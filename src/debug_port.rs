use crate::board::DbgUsart;
use crate::board::DbgUsartRx;
use crate::board::DbgUsartTx;
use crate::serial::Serial;
use embassy_stm32::usart::BufferedUart;
use embassy_stm32::{bind_interrupts, usart};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::signal::Signal;
use embedded_io::Read;
use heapless::Vec;
use core::cell::RefCell;

bind_interrupts!(struct Irqs {
        USART1 => usart::BufferedInterruptHandler<DbgUsart>;
});

pub struct DebugPort<'a, const BUFFER_SIZE: usize> {
    uart: BufferedUart<'a>,
    cmd_buffer: &'a Mutex<ThreadModeRawMutex, RefCell<[u8; 256]>>,
    cmd_signal: &'a Signal<ThreadModeRawMutex, bool>,
}

impl<'a, const BUFFER_SIZE: usize> DebugPort<'a, BUFFER_SIZE> {
    pub fn new(
        usart: DbgUsart,
        rx: DbgUsartRx,
        tx: DbgUsartTx,
        rx_buffer: &'a mut [u8],
        tx_buffer: &'a mut [u8],
        cmd_buffer: &'a Mutex<ThreadModeRawMutex, RefCell<[u8; 256]>>,
        cmd_signal: &'a Signal<ThreadModeRawMutex, bool>,
    ) -> Self {
        defmt::info!("debug_port_task started");

        let mut dbg_config = usart::Config::default();
        dbg_config.baudrate = 230400;

        Self {
            uart: BufferedUart::new(usart, Irqs, rx, tx, tx_buffer, rx_buffer, dbg_config).unwrap(),
            cmd_buffer,
            cmd_signal,
        }
    }

    pub async fn read_line(&mut self, into: &mut Vec<u8, BUFFER_SIZE>) {
        loop {
            let mut buf = [0; 128];
            let n = self.uart.read(&mut buf).unwrap();
            into.extend_from_slice(&buf[..n]).unwrap();
            self.cmd_buffer.lock(|cmd_buf| {
                cmd_buf.borrow_mut().copy_from_slice(&buf[..n]);
                self.cmd_signal.signal(true);
            });
        }
    }
}
