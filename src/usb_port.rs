use crate::board::UsbPeripherals;
use crate::line_breaker::LineBreaker;
use alloc::boxed::Box;
use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::gpio::Output;
use embassy_stm32::peripherals::USB;
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, usb};
use embassy_time::Timer;
use embassy_usb::class::cdc_acm;
use embassy_usb::{Builder, UsbDevice};
use embedded_io_async::Write;

bind_interrupts!(struct Irqs {
    USB_LP_CAN1_RX0 => usb::InterruptHandler<USB>;
});

const MAX_PACKET_SIZE: u8 = 64;

pub struct UsbPort {
    pub class: cdc_acm::CdcAcmClass<'static, Driver<'static, USB>>,
    breaker: LineBreaker,
    _usb_pullup: Output<'static>,
}

impl UsbPort {
    pub async fn new(mut usb_peripherals: UsbPeripherals, spawner: &'_ Spawner) -> UsbPort {
        info!("USB init");

        // Reset the USB D+ pin to simulate a disconnect, so we don't have to
        // manually disconnect the USB cable every time we upload new code.
        //
        usb_peripherals.usb_pullup.set_low();
        Timer::after_millis(100).await;
        usb_peripherals.usb_pullup.set_high();

        let driver = Driver::new(
            usb_peripherals.usb,
            Irqs,
            usb_peripherals.usb_dp,
            usb_peripherals.usb_dm,
        );

        let mut config = embassy_usb::Config::new(1155, 22336);
        config.manufacturer.replace("Walter's Basement");
        config.product.replace("Aunisoma Controller");
        config.serial_number.replace("00000001");
        config.max_power = 500;
        config.device_class = 0x02;
        config.device_sub_class = 0x02;
        config.device_protocol = 0x01;
        config.max_packet_size_0 = MAX_PACKET_SIZE;
        config.composite_with_iads = false;

        struct Resources<'r> {
            config_descriptor: [u8; 64],
            bos_descriptor: [u8; 16],
            control_buf: [u8; MAX_PACKET_SIZE as usize],
            serial_state: cdc_acm::State<'r>,
        }

        let resources = Box::leak(Box::new(Resources {
            config_descriptor: [0; 64],
            bos_descriptor: [0; 16],
            control_buf: [0; MAX_PACKET_SIZE as usize],
            serial_state: cdc_acm::State::new(),
        }));

        let mut builder = Builder::new(
            driver,
            config,
            &mut resources.config_descriptor,
            &mut resources.bos_descriptor,
            &mut [], // no msos descriptors
            &mut resources.control_buf,
        );

        let class = cdc_acm::CdcAcmClass::new(
            &mut builder,
            &mut resources.serial_state,
            MAX_PACKET_SIZE as u16,
        );

        let device = builder.build();
        spawner.must_spawn(driver_task(device));

        UsbPort {
            class,
            // This has to continue living, or else the pin will float.
            breaker: LineBreaker::new(256),
            _usb_pullup: usb_peripherals.usb_pullup,
        }
    }

    pub async fn read_line<'i>(&mut self, into: &'i mut [u8]) -> &'i [u8] {
        let mut buf = [0; MAX_PACKET_SIZE as usize];
        loop {
            self.class.wait_connection().await;
            loop {
                match self.class.read_packet(&mut buf).await {
                    Ok(n) => {
                        if let Some(line) = self.breaker.process(&buf[..n]) {
                            into[..line.len()].copy_from_slice(line);
                            return &into[..line.len()];
                        }
                    }
                    Err(e) => {
                        info!("USB read error: {}", e);
                        self.breaker.reset();
                        break;
                    }
                };
            }
        }
    }

    pub async fn write_line(&mut self, line: &[u8]) {
        let mut writer = CdcWriter::new(&mut self.class);
        writer.write_all(line).await;
        writer.write(b"\r").await;
        writer.flush().await;
    }
}

#[embassy_executor::task]
async fn driver_task(mut device: UsbDevice<'static, Driver<'static, USB>>) {
    device.run().await;
}

struct CdcWriter<'s, 'a> {
    class: &'s mut cdc_acm::CdcAcmClass<'a, Driver<'a, USB>>,
}

impl<'s, 'a> CdcWriter<'s, 'a> {
    fn new(class: &'s mut cdc_acm::CdcAcmClass<'a, Driver<'a, USB>>) -> Self {
        CdcWriter { class }
    }
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, defmt::Format)]
pub enum CdcWriterError {
    Other,
}

impl embedded_io::Error for CdcWriterError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

impl<'w, 'a> embedded_io::ErrorType for CdcWriter<'w, 'a> {
    type Error = CdcWriterError;
}

impl<'w, 'a> Write for CdcWriter<'w, 'a> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        match self.class.write_packet(buf).await {
            Ok(_) => Ok(buf.len()),
            Err(_) => Err(CdcWriterError::Other),
        }
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        let mut buf = buf;
        for chunk in buf.chunks(MAX_PACKET_SIZE as usize - 1) {
            match self.write(chunk).await {
                Ok(0) => core::panic!("write() returned Ok(0)"),
                Ok(n) => buf = &buf[n..],
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}
