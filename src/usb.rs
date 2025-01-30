use crate::board::{UsbDm, UsbDp};
use crate::line_breaker::LineBreaker;
use cortex_m::singleton;
use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::join;
use embassy_stm32::gpio::Output;
use embassy_stm32::peripherals::USB;
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usb};
use embassy_time::{Delay, Timer};
use embassy_usb::class::cdc_acm;
use embassy_usb::{Builder, UsbDevice};
use embedded_io_async::Write;
use heapless::Vec;

bind_interrupts!(struct Irqs {
    USB_LP_CAN1_RX0 => usb::InterruptHandler<peripherals::USB>;
});

const MAX_PACKET_SIZE: u8 = 64;

pub async fn init<const BUFFER_SIZE: usize>(
    spawner: Spawner,
    usb: USB,
    mut usb_pullup: Output<'static>,
    usb_dp: UsbDp,
    usb_dm: UsbDm,
) -> UsbSerial<'static, BUFFER_SIZE> {
    info!("USB init");

    // Reset the USB D+ pin to simulate a disconnect, so we don't have to
    // manually disconnect the USB cable every time we upload new code.
    //
    usb_pullup.set_low();
    Timer::after_millis(100).await;
    usb_pullup.set_high();

    let driver = Driver::new(usb, Irqs, usb_dp, usb_dm);

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

    struct Resources {
        config_descriptor: [u8; 64],
        bos_descriptor: [u8; 16],
        control_buf: [u8; MAX_PACKET_SIZE as usize],
        serial_state: cdc_acm::State<'static>,
    }

    let resources = singleton!(USB_RESOURCES: Resources = Resources {
        config_descriptor: [0; 64],
        bos_descriptor: [0; 16],
        control_buf: [0; MAX_PACKET_SIZE as usize],
        serial_state: cdc_acm::State::new(),
    })
    .unwrap();

    let mut builder = Builder::new(
        driver,
        config,
        &mut resources.config_descriptor,
        &mut resources.bos_descriptor,
        &mut [], // no msos descriptors
        &mut resources.control_buf,
    );

    let mut class = cdc_acm::CdcAcmClass::new(
        &mut builder,
        &mut resources.serial_state,
        MAX_PACKET_SIZE as u16,
    );

    let usb = builder.build();

    // join::join(driver_task(usb), command_task.run()).await;

    // WTF this works as an await but not as a task
    spawner.must_spawn(driver_task(usb));
    // spawner.must_spawn(command_task_x(class));

    Timer::after_millis(1000).await;
    UsbSerial::new(class)
}

#[embassy_executor::task]
async fn driver_task(mut device: UsbDevice<'static, Driver<'static, USB>>) {
    // async fn driver_task<'a>(mut device: UsbDevice<'a, Driver<'a, USB>>) {
    device.run().await;
}

#[embassy_executor::task]
async fn command_task_x(class: cdc_acm::CdcAcmClass<'static, Driver<'static, USB>>) {
    let mut command_task = CommandTask::new(class);
    command_task.run().await;
}

struct CommandTask<'a> {
    sender: cdc_acm::Sender<'a, Driver<'a, USB>>,
    receiver: cdc_acm::Receiver<'a, Driver<'a, USB>>,
}
impl<'a> CommandTask<'a> {
    fn new(class: cdc_acm::CdcAcmClass<'a, Driver<'a, USB>>) -> Self {
        let (sender, receiver) = class.split();
        Self { sender, receiver }
    }

    async fn run(&mut self) {
        loop {
            self.sender.wait_connection().await;
            let mut buf = [0; MAX_PACKET_SIZE as usize];
            loop {
                self.receiver.wait_connection().await;
                match self.receiver.read_packet(&mut buf).await {
                    Ok(n) => {
                        info!("{:?}", &buf[..n]);
                    }
                    Err(_) => {
                        info!("USB disconnected");
                        break;
                    }
                };
            }
        }
    }
}

struct CdcWriter<'s, 'a> {
    sender: &'s mut cdc_acm::Sender<'a, Driver<'a, USB>>,
}

impl<'s, 'a> CdcWriter<'s, 'a> {
    fn new(sender: &'s mut cdc_acm::Sender<'a, Driver<'a, USB>>) -> Self {
        CdcWriter { sender }
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
        match self.sender.write_packet(buf).await {
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

pub struct UsbSerial<'a, const BUFFER_SIZE: usize> {
    breaker: LineBreaker<BUFFER_SIZE>,
    class: cdc_acm::CdcAcmClass<'a, Driver<'a, USB>>,
}

impl<'a, const BUFFER_SIZE: usize> UsbSerial<'a, BUFFER_SIZE> {
    fn new(class: cdc_acm::CdcAcmClass<'a, Driver<'a, USB>>) -> Self {
        Self {
            breaker: LineBreaker::new(),
            class,
        }
    }

    pub async fn read_line(&mut self, into: &mut Vec<u8, BUFFER_SIZE>) {
        let mut buf = [0; MAX_PACKET_SIZE as usize];
        loop {
            self.class.wait_connection().await;
            info!("USB connected");
            let n = match self.class.read_packet(&mut buf).await {
                Ok(n) => n,
                Err(_) => {
                    info!("USB disconnected");
                    self.breaker.reset();
                    continue;
                }
            };
            if n == 0 {
                continue;
            }
            if let Some(line) = self.breaker.process(&buf[..n]) {
                into.extend_from_slice(line).unwrap();
            }
        }
    }
}
