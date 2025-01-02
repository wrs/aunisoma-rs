use crate::board::{UsbDm, UsbDp};
use defmt::{error, info, warn, Format};
use embassy_futures::join::join;
use embassy_stm32::gpio::Output;
use embassy_stm32::peripherals::USB;
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usb};
use embassy_time::Timer;
use embassy_usb::class::cdc_acm;
use embassy_usb::{Builder, UsbDevice};
use heapless::Vec;

bind_interrupts!(struct Irqs {
    USB_LP_CAN1_RX0 => usb::InterruptHandler<peripherals::USB>;
});

const MAX_PACKET_SIZE: u8 = 64;

#[embassy_executor::task]
pub async fn usb_task(usb: USB, mut usb_pullup: Output<'static>, usb_dp: UsbDp, usb_dm: UsbDm) {
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

    let mut device_descriptor: [u8; 256] = [0; 256];
    let mut config_descriptor: [u8; 256] = [0; 256];
    let mut control_buf: [u8; MAX_PACKET_SIZE as usize] = [0; MAX_PACKET_SIZE as usize];
    let mut serial_state: cdc_acm::State = cdc_acm::State::new();

    let mut builder = Builder::new(
        driver,
        config,
        &mut device_descriptor,
        &mut config_descriptor,
        &mut [], // no msos descriptors
        &mut control_buf,
    );

    let class = cdc_acm::CdcAcmClass::new(&mut builder, &mut serial_state, MAX_PACKET_SIZE as u16);

    let usb = builder.build();

    let mut command_task = CommandTask::new(class);
    join(driver_task(usb), command_task.run()).await;
}

async fn driver_task<'a>(mut device: UsbDevice<'a, Driver<'a, USB>>) {
    device.run().await;
}

struct CommandTask<'a> {
    class: cdc_acm::CdcAcmClass<'a, Driver<'a, USB>>,
}

impl<'a> CommandTask<'a> {
    fn new(class: cdc_acm::CdcAcmClass<'a, Driver<'a, USB>>) -> Self {
        Self { class }
    }

    async fn run(&mut self) {
        loop {
            self.class.wait_connection().await;
            info!("USB connected");

            let mut reader = LineBreaker::<128>::new();
            let mut buf = [0; MAX_PACKET_SIZE as usize];
            while let Ok(n) = self.class.read_packet(&mut buf).await {
                if n == 0 {
                    break;
                }
                let result = reader.process(&buf[..n]).await;
                if let Some(line) = result {
                    info!("data: {:?}", core::str::from_utf8(line).unwrap());
                    for chunk in line.chunks(MAX_PACKET_SIZE as usize - 1) {
                        if let Err(e) = self.class.write_packet(chunk).await {
                            error!("USB error: {}", e);
                            break;
                        }
                    }
                }
            }

            info!("USB disconnected");
        }
    }
}

struct LineBreaker<const N: usize> {
    buffer: Vec<u8, N>,
    used_prefix: usize,
    discard: bool,
}

impl<const N: usize> LineBreaker<N> {
    fn new() -> Self {
        Self {
            buffer: Vec::<u8, N>::new(),
            used_prefix: 0,
            discard: false,
        }
    }

    // This works best if buf is at least 2*MAX_PACKET_SIZE

    async fn process(&mut self, buf: &[u8]) -> Option<&[u8]> {
        info!("buf: {} used_prefix: {} discard: {}", core::str::from_utf8(buf).unwrap(), self.used_prefix, self.discard);
        if self.used_prefix > 0 {
            let len = self.buffer.len();
            self.buffer.copy_within(self.used_prefix..len, 0);
            assert!(self.buffer.resize(len - self.used_prefix, 0).is_ok());
            self.used_prefix = 0;
        }

        if buf.len() == 0 {
            return None;
        }

        let mut split = buf.splitn(2, |b| *b == b'\n');
        // We know buf is not empty, so unwrap is safe
        let first = split.next().unwrap();
        let rest = split.next();
        info!("first={:?} rest={:?}", first, rest);

        if let Some(rest) = rest {
            if self.discard {
                // Discard the (partial) current line
                self.buffer.clear();
                // Save the beginning of the next line
                assert!(
                    self.buffer.extend_from_slice(rest).is_ok(),
                    "No room for line fragment"
                );
                self.discard = false;
                return None;
            }

            // Save the end of the current line
            if self.buffer.extend_from_slice(first).is_ok() {
                let line_len = self.buffer.len();
                if self.buffer.extend_from_slice(rest).is_ok() {
                    // We saved the beginning of the next line, yay happy path!
                    self.used_prefix = line_len;
                    return Some(&self.buffer[..line_len]);
                }
                // We didn't have room for the beginning of the next line, so
                // discard the rest of it.
                self.discard = true;
                self.used_prefix = line_len;
                return Some(&self.buffer[..line_len]);
            } else {
                // Line too long, discard it
                self.buffer.clear();
                self.discard = true;
                return None;
            }
        } else {
            // No line ending found, so just append the buffer
            if self.buffer.extend_from_slice(first).is_ok() {
                return None;
            }
            // Line too long, discard it
            self.buffer.clear();
            self.discard = true;
            return None;
        }
    }
}
// ********************************************************************************************************************************
