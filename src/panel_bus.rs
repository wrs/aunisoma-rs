use crate::board::{PanelBusUsart, PanelBusUsartRxDma, PanelBusUsartTx, PanelBusUsartTxDma};
use crate::comm::{Address, ReceiveCallback, MAX_PAYLOAD_SIZE};
use embassy_stm32::gpio::Output;
use embassy_stm32::mode::Async;
use embassy_stm32::usart::{HalfDuplexConfig, HalfDuplexReadback, Uart};
use embassy_stm32::{bind_interrupts, usart};
use embedded_io_async::Write;
use heapless::Vec;

bind_interrupts!(struct Irqs {
        USART2 => usart::InterruptHandler<PanelBusUsart>;
});

pub struct PanelBus<'a> {
    address: Address,
    receive_callback: Option<ReceiveCallback>,
    uart: Uart<'a, Async>,
    ser_out_en: Output<'static>,
}

pub enum PanelBusError {
    Usart(usart::Error),
}

impl<'a> PanelBus<'a> {
    pub async fn new(
        address: Address,
        receive_callback: Option<ReceiveCallback>,
        usart: PanelBusUsart,
        tx: PanelBusUsartTx,
        tx_dma: PanelBusUsartTxDma,
        rx_dma: PanelBusUsartRxDma,
        mut ser_out_en: Output<'static>,
    ) -> Self {
        defmt::info!("panel_bus_task started");

        let mut dbg_config = usart::Config::default();
        dbg_config.baudrate = 230400;

        ser_out_en.set_low();

        let uart = Uart::new_half_duplex(
            usart,
            tx,
            Irqs,
            tx_dma,
            rx_dma,
            dbg_config,
            HalfDuplexReadback::NoReadback,
            HalfDuplexConfig::PushPull,
        )
        .unwrap();

        PanelBus {
            address,
            receive_callback,
            uart,
            ser_out_en,
        }
    }

    pub async fn read(&mut self) -> Result<u8, usart::Error> {
        let mut buf = [0; 1];
        self.uart.read(&mut buf).await.unwrap();
        Ok(buf[0])
    }

    pub async fn write(&mut self, data: u8) -> Result<(), usart::Error> {
        let buf = [data];
        self.uart.write_all(&buf).await.unwrap();
        Ok(())
    }

    fn write_all_with_en(&mut self, buf: &[u8]) -> Result<(), usart::Error> {
        self.ser_out_en.set_high();
        let result = embassy_futures::block_on(self.uart.write_all(buf));
        self.ser_out_en.set_low();
        result
    }

    pub fn send_to(&mut self, to_addr: Address, data: &[u8]) -> Result<(), PanelBusError> {
        let mut packet = Vec::<u8, MAX_PAYLOAD_SIZE>::new();
        packet.push(to_addr.value());
        packet.extend_from_slice(data);
        self.write_all_with_en(&packet).map_err(PanelBusError::Usart)
    }
}
