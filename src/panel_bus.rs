use crate::board::PanelBusUsart;
use crate::board::PanelBusUsartTx;
use crate::board::PanelBusUsartTxDma;
use crate::board::PanelBusUsartRxDma;
use embassy_stm32::usart::HalfDuplexConfig;
use embassy_stm32::usart::Uart;
use embassy_stm32::{bind_interrupts, usart};
use embedded_io_async::Write;

bind_interrupts!(struct Irqs {
        USART2 => usart::InterruptHandler<PanelBusUsart>;
});

#[embassy_executor::task]
pub(crate) async fn panel_bus_task(
    usart: PanelBusUsart,
    tx: PanelBusUsartTx,
    tx_dma: PanelBusUsartTxDma,
    rx_dma: PanelBusUsartRxDma,
) {
    defmt::info!("panel_bus_task started");

    let mut dbg_config = usart::Config::default();
    dbg_config.baudrate = 230400;

    let mut usart_bus = Uart::new_half_duplex(
        usart,
        tx,
        Irqs,
        tx_dma,
        rx_dma,
        dbg_config,
        HalfDuplexConfig::PushPull,
    )
    .unwrap();

    loop {
        let mut buf = [0; 1];
        usart_bus.read(&mut buf).await.unwrap();
        usart_bus.write_all(&buf).await.unwrap();
    }
}
