use crate::board::{UsbDm, UsbDp};
use defmt::*;
use embassy_futures::join::join;
use embassy_stm32::gpio::Output;
use embassy_stm32::peripherals::USB;
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usb};
use embassy_time::Timer;
use embassy_usb::class::cdc_acm;
use embassy_usb::{Builder, UsbDevice};

bind_interrupts!(struct Irqs {
    USB_LP_CAN1_RX0 => usb::InterruptHandler<peripherals::USB>;
});

#[embassy_executor::task]
pub async fn usb_task(usb: USB, mut usb_pullup: Output<'static>, usb_dp: UsbDp, usb_dm: UsbDm) {
    info!("USB init");

    // Reset the USB D+ pin to simulate a disconnect, so we don't have to
    // manually disconnect the USB cable every time we upload new code.
    //
    trace!("USB D+ reset");
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
    config.max_packet_size_0 = 64;

    let mut device_descriptor: [u8; 256] = [0; 256];
    let mut config_descriptor: [u8; 256] = [0; 256];
    let mut control_buf: [u8; 64] = [0; 64];
    let mut serial_state: cdc_acm::State = cdc_acm::State::new();

    let mut builder = Builder::new(
        driver,
        config,
        &mut device_descriptor,
        &mut config_descriptor,
        &mut [], // no msos descriptors
        &mut control_buf,
    );

    let class = cdc_acm::CdcAcmClass::new(&mut builder, &mut serial_state, 64);

    let usb = builder.build();

    join(driver_task(usb), echo_task(class)).await;
}

async fn driver_task<'a>(mut device: UsbDevice<'a, Driver<'a, USB>>) {
    device.run().await;
}

async fn echo_task<'a>(mut class: cdc_acm::CdcAcmClass<'a, Driver<'a, USB>>) {
    loop {
        class.wait_connection().await;
        info!("Connected");
        let mut buf = [0; 64];
        loop {
            match class.read_packet(&mut buf).await {
                Ok(n) => {
                    let data = &buf[..n];
                    info!("data: {:x}", data);
                    if let Err(e) = class.write_packet(data).await {
                        info!("{}", e);
                        break;
                    }
                }
                Err(_e) => {
                    info!("{}", _e);
                    break;
                }
            }
        }
        info!("Disconnected");
    }
}
