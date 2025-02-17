use crate::{
    board::{PanelBusPeripherals, PanelBusUsart, RadioPeripherals},
    cmd_processor::Message,
};
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

type PacketData = heapless::Vec<u8, { MAX_PAYLOAD_SIZE }>;

/// Internal representation of a packet
///
/// The wire format of a packet is a little goofy because it's
/// backwards-compatible with the C++ version:
///
/// [0x55, 0xaa, to, data_len+2, from, tag, data*, crc]
///
/// For this struct, only to, from, tag, and data are stored, the rest are calculated
/// when the packet is serialized. So self.data is:
///
/// [to, data_len+2, from, tag, data*]
///
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Packet {
    pub from: Address,
    pub to: Address,
    pub tag: u8,
    pub data: PacketData,
}

impl Packet {
    pub fn new(from: Address, to: Address, tag: u8) -> Self {
        Self {
            from,
            to,
            tag,
            data: PacketData::new(),
        }
    }

    pub fn push_data(&mut self, data: &[u8]) {
        self.data.extend_from_slice(data).unwrap();
    }

    /// Write the packet to a buffer in wire format.
    ///
    /// The buffer must be at least MAX_PAYLOAD_SIZE + 8 bytes long.
    ///
    pub fn wire_format<'a>(&self, buf: &'a mut [u8]) -> &'a [u8] {
        buf[0..6].copy_from_slice(&[
            0x55,
            0xaa,
            self.to.value(),
            self.data.len() as u8 + 2,
            self.from.value(),
            self.tag,
        ]);
        buf[6..6 + self.data.len()].copy_from_slice(&self.data);
        // TODO: calculate crc
        buf[6 + self.data.len() + 1] = b'C';
        &buf[..6 + self.data.len() + 2]
    }
}

impl defmt::Format for Packet {
    fn format(&self, fmt: defmt::Formatter<'_>) {
        let data = self.data.as_slice();
        defmt::write!(
            fmt,
            "({:x} -> {:x}) {} {:02x}",
            self.from.value(),
            self.to.value(),
            self.tag as char,
            data
        );
    }
}

pub enum CommMode {
    Radio,
    Serial,
}

pub struct PanelComm {
    mode: CommMode,
    from: Address,
    radio: PanelRadio,
    serial: PanelSerial,
}

impl PanelComm {
    pub fn new(mode: CommMode, from: Address, radio: PanelRadio, serial: PanelSerial) -> Self {
        Self {
            mode,
            from,
            radio,
            serial,
        }
    }

    pub async fn send_packet(&mut self, packet: &Packet) {
        debug!("Sending packet: {:?}", packet);
        match self.mode {
            CommMode::Radio => self.radio.send_packet(packet).await,
            CommMode::Serial => self.serial.send_packet(packet).await,
        }
    }

    pub async fn recv_packet(&mut self) -> Packet {
        match self.mode {
            CommMode::Radio => self.radio.recv_packet().await,
            CommMode::Serial => self.serial.recv_packet().await,
        }
    }
}

pub struct PanelRadio {}

impl PanelRadio {
    pub fn new(radio_peripherals: RadioPeripherals) -> Self {
        Self {}
    }

    pub async fn send_packet(&mut self, packet: &Packet) {
        todo!()
    }

    pub async fn recv_packet(&mut self) -> Packet {
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

        let (mut tx, rx) = uart.split();

        // Without this, the first write doesn't happen until the second write
        embedded_io::Write::write(&mut tx, &[]).unwrap();

        Self {
            ser_out_en: panel_bus_peripherals.ser_out_en,
            tx,
            rx,
            address,
        }
    }

    pub async fn send_packet(&mut self, packet: &Packet) {
        if packet.data.len() > MAX_PAYLOAD_SIZE {
            error!("Data length too long");
            return;
        }

        let mut buf = [0u8; MAX_PAYLOAD_SIZE + 8];
        let wire_data = packet.wire_format(&mut buf);
        // debug!("Wire format: {:x}", wire_data);

        self.ser_out_en.set_high();

        // Need to manually enable the transmitter
        // https://github.com/embassy-rs/embassy/pull/3679#issuecomment-2662106197
        embassy_stm32::pac::USART2.cr1().modify(|w| {
            w.set_re(false);
            w.set_te(true);
        });

        if self.tx.write_all(wire_data).await.is_err() {
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

    // TODO: mid-packet timeout
    // TODO: crc check
    // TODO: could we just receive until idle?

    pub async fn recv_packet(&mut self) -> Packet {
        loop {
            while self.read_byte().await != 0x55 {}
            if self.read_byte().await != 0xaa {
                continue;
            }
            let to = self.read_byte().await;
            let len = self.read_byte().await as usize;
            if !(2..=MAX_PAYLOAD_SIZE + 2).contains(&len) {
                // +2 for from and tag
                continue;
            }
            let from = Address(self.read_byte().await);
            let tag = self.read_byte().await;

            let data_len = len - 2; // Subtract from and tag
            let mut packet = Packet::new(from, Address(to), tag);

            if data_len > 0 {
                let _ = packet.data.resize(data_len, 0);
                if self.rx.read_exact(&mut packet.data[0..data_len]).await.is_err() {
                    continue;
                }
            }

            let crc = self.read_byte().await;
            // TODO: real crc check
            if crc != b'C' {
                error!("CRC error: {:02x}", crc);
                continue;
            }

            debug!("Received packet: {:?}", packet);

            if to == BROADCAST_ADDRESS.value() || to == self.address.value() {
                return packet;
            }
        }
    }

    async fn read_byte(&mut self) -> u8 {
        let mut buffer = [0; 1];
        embassy_stm32::pac::GPIOB.bsrr().write(|w| w.set_bs(13, true));
        if let Err(e) = self.rx.read(&mut buffer).await {
            error!("read_byte error: {:?}", e);
        }
        embassy_stm32::pac::GPIOB.bsrr().write(|w| w.set_br(13, true));
        // debug!("Received: {:02x}", buffer[0]);
        buffer[0]
    }
}
