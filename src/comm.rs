use crate::board::{CmdPortPeripherals, PanelBusPeripherals, PanelBusUsart, RadioPeripherals};
use alloc::boxed::Box;
use defmt::{debug, error};
use embassy_stm32::{
    bind_interrupts,
    gpio::Output,
    mode::Async,
    usart::{self, HalfDuplexConfig, HalfDuplexReadback, Uart},
};
use embedded_io::Write;

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
    pub fn new(mode: CommMode, address: Address, radio: PanelRadio, serial: PanelSerial) -> Self {
        Self {
            mode,
            radio,
            serial,
        }
    }

    pub async fn send_packet(&mut self, to: Address, data: &[u8]) {
        debug!("Sending packet to {:x}: {:x}", to.value(), data);
        match self.mode {
            CommMode::Radio => self.radio.send_packet(to, data).await,
            CommMode::Serial => self.serial.send_packet(to, data).await,
        }
    }

    pub async fn recv_packet<'a>(&mut self, buffer: &'a mut [u8]) -> &'a [u8] {
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

    pub async fn send_packet(&mut self, to: Address, data: &[u8]) {
        todo!()
    }

    pub async fn recv_packet<'a>(&mut self, buffer: &'a mut [u8]) -> &'a [u8] {
        todo!()
    }
}

pub struct PanelSerial {
    ser_out_en: Output<'static>,
    tx: usart::UartTx<'static, Async>,
    rx: usart::UartRx<'static, Async>,
    address: Address,
}

impl PanelSerial {
    pub fn new(mut panel_bus_peripherals: PanelBusPeripherals, address: Address) -> Self {
        let mut config = usart::Config::default();
        config.baudrate = 1_000_000;

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

        let (tx, rx) = uart.split();

        Self {
            ser_out_en: panel_bus_peripherals.ser_out_en,
            tx,
            rx,
            address,
        }
    }

    pub async fn send_packet(&mut self, to: Address, data: &[u8]) {
        if data.len() > MAX_PAYLOAD_SIZE {
            error!("Data length too long");
            return;
        }
        self.ser_out_en.set_high();
        self.tx
            .write_all(&[0x55, 0xaa, to.value(), data.len() as u8])
            .unwrap();
        self.tx.write_all(data).unwrap();
        self.tx.write(b"C").await.unwrap();
        self.tx.flush().await.unwrap();
        self.ser_out_en.set_low();
    }

    pub async fn recv_packet<'a>(&mut self, buffer: &'a mut [u8]) -> &'a [u8] {
        loop {
            while self.read_byte().await != 0x55 {}
            if self.read_byte().await != 0xaa {
                continue;
            }
            let to = self.read_byte().await;
            let len = self.read_byte().await as usize;
            if len > buffer.len() {
                continue;
            }
            if self.rx.read(&mut buffer[0..len as usize]).await.is_err() {
                continue;
            };
            let crc = self.read_byte().await;
            // TODO: check crc
            if to == BROADCAST_ADDRESS.value() || to == self.address.value() {
                return &buffer[0..len as usize];
            }
        }
    }

    async fn read_byte(&mut self) -> u8 {
        let mut buffer = [0; 1];
        match self.rx.read(&mut buffer).await {
            Ok(_) => debug!("Rcvd: 0x{:x}", buffer[0]),
            Err(e) => error!("Error reading byte: {:?}", e),
        }
        buffer[0]
    }
}
