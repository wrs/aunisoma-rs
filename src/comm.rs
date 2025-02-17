use crate::board::{PanelBusPeripherals, PanelBusUsart, RadioPeripherals};
use alloc::boxed::Box;
use defmt::{debug, error};
use embassy_stm32::{
    bind_interrupts,
    gpio::Output,
    usart::{self, BufferedUart, HalfDuplexConfig, HalfDuplexReadback},
};
use embedded_io_async::{Read, Write};

bind_interrupts!(struct Irqs {
    USART2 => usart::BufferedInterruptHandler<PanelBusUsart>;
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

type PacketData = heapless::Vec<u8, { MAX_PAYLOAD_SIZE + 5 }>;

/// Internal representation of a packet
///
/// The wire format of a packet is:
/// [from, len, data*, crc]
///
#[derive(Debug, PartialEq, Eq, Clone)]
struct Packet {
    from: Address,
    data: PacketData,
}

impl Packet {
    pub fn new(from: Address) -> Self {
        Self {
            from,
            data: PacketData::new(),
        }
    }

    pub fn from_wire(wire: &[u8]) -> Self {
        Self {
            from: Address(wire[0]),
            data: PacketData::from_slice(&wire[1..]).unwrap(),
        }
    }

    pub fn push_data(&mut self, data: &[u8]) {
        self.data.extend_from_slice(data).unwrap();
    }

    pub fn to_wire(self) -> [u8; MAX_PAYLOAD_SIZE + 5] {
        let mut wire = [0; MAX_PAYLOAD_SIZE + 5];
        wire[0] = self.from.value();
        wire[1] = self.data.len() as u8;
        wire[2..self.data.len() + 2].copy_from_slice(&self.data);
        wire
    }
}

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
    tx: usart::BufferedUartTx<'static>,
    rx: usart::BufferedUartRx<'static>,
    address: Address,
}

impl PanelSerial {
    pub fn new(mut panel_bus_peripherals: PanelBusPeripherals, address: Address) -> Self {
        let mut config = usart::Config::default();
        config.baudrate = 256_000;

        panel_bus_peripherals.ser_out_en.set_low();

        let rx_buffer = Box::leak(Box::new([0; 256]));
        let tx_buffer = Box::leak(Box::new([0; 256]));

        let uart = BufferedUart::new_half_duplex(
            panel_bus_peripherals.panel_bus_usart,
            panel_bus_peripherals.panel_bus_usart_tx,
            Irqs,
            tx_buffer,
            rx_buffer,
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
        let mut buf = heapless::Vec::<u8, { MAX_PAYLOAD_SIZE + 5 }>::new();
        buf.extend_from_slice(&[0x55, 0xaa, to.value(), data.len() as u8])
            .unwrap();
        buf.extend_from_slice(data).unwrap();
        // TODO: calculate crc
        let crc = b'C';
        buf.push(crc).unwrap();
        self.ser_out_en.set_high();

        // Need to manually enable the transmitter
        // https://github.com/embassy-rs/embassy/pull/3679#issuecomment-2662106197
        embassy_stm32::pac::USART2.cr1().modify(|w| {
            w.set_re(false);
            w.set_te(true);
        });

        if self.tx.write_all(&buf).await.is_err() {
            error!("Error writing packet");
        }
        self.tx.flush().await.unwrap();
        self.ser_out_en.set_low();

        // Need to manually enable the receiver after tx is done
        embassy_stm32::pac::USART2.cr1().modify(|w| {
            w.set_re(true);
            w.set_te(false);
        });
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
            // TODO: real crc check
            if crc != b'C' {
                error!("CRC error");
                continue;
            }
            debug!("to {:x}: {:x}", to, &buffer[0..len as usize]);
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
