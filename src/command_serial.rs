use crate::board::CmdPortPeripherals;
use crate::board::DbgUsart;
use crate::line_breaker::LineBreaker;
use alloc::boxed::Box;
use defmt::info;
use embassy_stm32::usart::BufferedUart;
use embassy_stm32::{bind_interrupts, usart};
use embedded_io_async::{Read, Write};

bind_interrupts!(struct Irqs {
        USART1 => usart::BufferedInterruptHandler<DbgUsart>;
});

pub struct CommandSerial<'a> {
    uart: BufferedUart<'a>,
    breaker: LineBreaker<256>,
}

impl CommandSerial<'_> {
    pub fn new(p: CmdPortPeripherals) -> Self {
        let mut config = usart::Config::default();
        config.baudrate = 230400;

        let rx_buffer = Box::leak(Box::new([0; 256]));
        let tx_buffer = Box::leak(Box::new([0; 256]));

        Self {
            uart: BufferedUart::new(
                p.cmd_usart,
                Irqs,
                p.cmd_usart_rx,
                p.cmd_usart_tx,
                tx_buffer,
                rx_buffer,
                config,
            )
            .unwrap(),
            breaker: LineBreaker::new(),
        }
    }

    pub async fn read_line<'i>(&mut self, into: &'i mut [u8]) -> &'i [u8] {
        loop {
            let mut buf = [0; 128];
            match self.uart.read(&mut buf).await {
                Ok(n) => {
                    if let Some(line) = self.breaker.process(&buf[..n]) {
                        into[..line.len()].copy_from_slice(line);
                        return &into[..line.len()];
                    }
                }
                Err(e) => {
                    info!("UART read error: {}", e);
                    return &[];
                }
            }
        }
    }

    pub async fn write_line(&mut self, line: &[u8]) {
        let _ = self.uart.write_all(line).await;
        let _ = self.uart.write(b"\n").await;
        let _ = self.uart.flush().await;
    }
}
