use crate::board::PanelBusUsart;
use crate::board::PanelBusUsartRxDma;
use crate::board::PanelBusUsartTx;
use crate::board::PanelBusUsartTxDma;
use embassy_stm32::gpio::Output;
use embassy_stm32::mode::Async;
use embassy_stm32::usart::HalfDuplexConfig;
use embassy_stm32::usart::Uart;
use embassy_stm32::{bind_interrupts, usart};
use embedded_io_async::Write;

bind_interrupts!(struct Irqs {
        USART2 => usart::InterruptHandler<PanelBusUsart>;
});

#[embassy_executor::task]
pub async fn panel_bus_task(
    usart: PanelBusUsart,
    tx: PanelBusUsartTx,
    tx_dma: PanelBusUsartTxDma,
    rx_dma: PanelBusUsartRxDma,
    mut ser_out_en: Output<'static>,
) {
    defmt::info!("panel_bus_task started");

    let mut dbg_config = usart::Config::default();
    dbg_config.baudrate = 230400;

    ser_out_en.set_low();

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

    let mut buf = [0; 1];
    loop {
        usart_bus.read(&mut buf).await.unwrap();

        write_all_with_en(&mut usart_bus, &buf, &mut ser_out_en)
            .await
            .unwrap();
    }
}

async fn write_all_with_en(
    usart: &mut Uart<'_, Async>,
    buf: &[u8],
    ser_out_en: &mut Output<'static>,
) -> Result<(), usart::Error> {
    ser_out_en.set_high();
    let result = usart.write_all(buf).await;
    ser_out_en.set_low();
    result
}
