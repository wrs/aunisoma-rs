use crate::board::{CmdPortPeripherals, PanelBusPeripherals, PanelBusUsart, RadioPeripherals};
use defmt::debug;
use embassy_stm32::{
    bind_interrupts,
    gpio::Output,
    mode::Async,
    usart::{self, HalfDuplexConfig, HalfDuplexReadback, Uart},
};

bind_interrupts!(struct Irqs {
    USART2 => usart::InterruptHandler<PanelBusUsart>;
});

pub const MAX_PAYLOAD_SIZE: usize = 64;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Address(pub u8);

impl Address {
    pub fn value(&self) -> u8 {
        self.0
    }
}

pub const BROADCAST_ADDRESS: Address = Address(0xFF);

pub enum CommMode {
    Radio,
    Serial,
}

pub struct PanelComm {
    mode: CommMode,
    radio: PanelRadio,
    serial: PanelSerial,
}

impl PanelComm {
    pub fn new(
        mode: CommMode,
        radio: PanelRadio,
        serial: PanelSerial,
    ) -> Self {
        Self {
            mode,
            radio,
            serial,
        }
    }

    pub async fn send_packet(&mut self, data: &[u8]) {
        debug!("Sending packet: {:x}", data);
        match self.mode {
            CommMode::Radio => self.radio.send_packet(data).await,
            CommMode::Serial => self.serial.send_packet(data).await,
        }
    }

    pub async fn recv_packet(&mut self, buffer: &mut [u8]) -> &[u8] {
        match self.mode {
            CommMode::Radio => self.radio.recv_packet(buffer).await,
            CommMode::Serial => self.serial.recv_packet(buffer).await,
        }
    }
}

pub struct PanelRadio {}

impl PanelRadio {
    pub fn new(radio_peripherals: RadioPeripherals) -> Self {
        Self {}
    }

    pub async fn send_packet(&mut self, data: &[u8]) {
        todo!()
    }

    pub async fn recv_packet(&mut self, buffer: &mut [u8]) -> &[u8] {
        todo!()
    }
}

pub struct PanelSerial {
    ser_out_en: Output<'static>,
    usart: usart::Uart<'static, Async>,
}

impl PanelSerial {
    pub fn new(mut panel_bus_peripherals: PanelBusPeripherals) -> Self {
        let mut config = usart::Config::default();
        config.baudrate = 230400;

        panel_bus_peripherals.ser_out_en.set_low();

        let uart = Uart::new_half_duplex(
            panel_bus_peripherals.panel_bus_usart,
            panel_bus_peripherals.panel_bus_usart_tx,
            Irqs,
            panel_bus_peripherals.panel_bus_usart_tx_dma,
            panel_bus_peripherals.panel_bus_usart_rx_dma,
            config,
            HalfDuplexReadback::NoReadback,
            HalfDuplexConfig::PushPull,
        )
        .unwrap();

        Self {
            ser_out_en: panel_bus_peripherals.ser_out_en,
            usart: uart,
        }
    }

    pub async fn send_packet(&mut self, data: &[u8]) {
        self.usart.write(data).await.unwrap();
    }

    pub async fn recv_packet(&mut self, buffer: &mut [u8]) -> &[u8] {
        b"0"
    }
}
