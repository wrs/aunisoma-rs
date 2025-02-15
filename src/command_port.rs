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

pub struct CommandPort<'a> {
    uart: BufferedUart<'a>,
    breaker: LineBreaker,
}

impl CommandPort<'_> {
    pub fn new(p: CmdPortPeripherals) -> Self {
        let mut dbg_config = usart::Config::default();
        dbg_config.baudrate = 230400;

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
                dbg_config,
            )
            .unwrap(),
            breaker: LineBreaker::new(256),
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
        self.uart.write_all(line).await;
        self.uart.write(b"\r").await;
        self.uart.flush().await;
    }
}
