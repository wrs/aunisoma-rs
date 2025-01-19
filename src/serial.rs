use heapless::Vec;

use crate::{debug_port::DebugPort, usb::UsbSerial};

pub enum Serial<'a, const BUFFER_SIZE: usize> {
    DebugPort(DebugPort<'a, BUFFER_SIZE>),
    UsbSerial(UsbSerial<'a, BUFFER_SIZE>),
}

impl<'a, const BUFFER_SIZE: usize> Serial<'a, BUFFER_SIZE> {
    pub async fn read_line<'b>(&mut self, into: &'b mut Vec<u8, BUFFER_SIZE>) {
        match self {
            Serial::DebugPort(debug_port) => debug_port.read_line(into).await,
            Serial::UsbSerial(usb_serial) => usb_serial.read_line(into).await,
        }
    }
}
