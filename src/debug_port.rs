use crate::board::DbgUsart;
use crate::board::DbgUsartRx;
use crate::board::DbgUsartTx;
use embassy_stm32::usart::BufferedUart;
use embassy_stm32::{bind_interrupts, usart};
use embedded_io_async::Read;
use embedded_io_async::Write;

bind_interrupts!(struct Irqs {
        USART1 => usart::BufferedInterruptHandler<DbgUsart>;
});

#[embassy_executor::task]
pub(crate) async fn debug_port_task(usart: DbgUsart, rx: DbgUsartRx, tx: DbgUsartTx) {
    defmt::info!("debug_port_task started");

    let mut tx_buffer: [u8; 128] = [0; 128];
    let mut rx_buffer: [u8; 128] = [0; 128];

    let mut dbg_config = usart::Config::default();
    dbg_config.baudrate = 230400;

    let mut uart = BufferedUart::new(
        usart,
        Irqs,
        rx,
        tx,
        tx_buffer.as_mut_slice(),
        rx_buffer.as_mut_slice(),
        dbg_config,
    )
    .unwrap();

    loop {
        let mut buf = [0; 128];
        let n = uart.read(&mut buf).await.unwrap();
        let _ = uart.write_all(&buf[..n]).await;
    }
}
