use core::convert::Infallible;

use crate::{
    board::{PanelBusPeripherals, PanelBusUsart, RadioPeripherals},
    cmd_processor::Message,
};
use alloc::boxed::Box;
use defmt::{debug, error, info, Format};
use embassy_stm32::{
    bind_interrupts,
    exti::ExtiInput,
    gpio::{Output, Pull},
    mode::Blocking,
    spi::{self, Spi},
    usart::{self, BufferedUart, HalfDuplexConfig, HalfDuplexReadback},
};
use embassy_time::Timer;
use embedded_hal_bus::spi::{DeviceError, ExclusiveDevice, NoDelay};
use embedded_io_async::{Read, Write};
use rfm69::{Rfm69, registers};

bind_interrupts!(struct Irqs {
    USART2 => usart::BufferedInterruptHandler<PanelBusUsart>;
});

pub const MAX_PAYLOAD_SIZE: usize = 61;

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
    pub tag: Message,
    pub data: PacketData,
}

impl Packet {
    pub fn new(from: Address, to: Address, tag: Message) -> Self {
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
    pub fn serial_wire_format<'a>(&self, buf: &'a mut [u8]) -> &'a [u8] {
        buf[0..6].copy_from_slice(&[
            0x55,
            0xaa,
            self.to.value(),
            self.data.len() as u8 + 2,
            self.from.value(),
            self.tag.into(),
        ]);
        buf[6..6 + self.data.len()].copy_from_slice(&self.data);
        // TODO: calculate crc
        buf[6 + self.data.len()] = b'C';
        &buf[..6 + self.data.len() + 1]
    }

    pub fn radio_wire_format<'a>(&self, buf: &'a mut [u8]) -> &'a [u8] {
        buf[0..4].copy_from_slice(&[
            self.data.len() as u8 + 3,
            self.to.value(),
            self.from.value(),
            self.tag.into(),
        ]);
        buf[4..4 + self.data.len()].copy_from_slice(&self.data);
        &buf[..4 + self.data.len()]
    }
}

impl defmt::Format for Packet {
    fn format(&self, fmt: defmt::Formatter<'_>) {
        let data = self.data.as_slice();
        defmt::write!(
            fmt,
            "({:x} -> {:x}) {:a} {:02x}",
            self.from.value(),
            self.to.value(),
            self.tag as u8 as char,
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
    radio: PanelRadio,
    serial: PanelSerial,
}

impl PanelComm {
    pub fn new(mode: CommMode, radio: PanelRadio, serial: PanelSerial) -> Self {
        Self {
            mode,
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

    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            CommMode::Radio => "Radio",
            CommMode::Serial => "Serial",
        }
    }
}

#[derive(Format)]
pub enum RadioError {
    Rfm69,
    NoRadio,
    NoPacketAvailable,
    InvalidPacket,
}

impl From<rfm69::Error<DeviceError<embassy_stm32::spi::Error, Infallible>>> for RadioError {
    fn from(_: rfm69::Error<DeviceError<embassy_stm32::spi::Error, Infallible>>) -> Self {
        RadioError::Rfm69
    }
}

type RadioResult<T> = Result<T, RadioError>;
pub struct PanelRadio {
    radio: Rfm69<ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, NoDelay>>,
    reset: Output<'static>,
    dio_int: ExtiInput<'static>,
}

impl PanelRadio {
    const FREQUENCY: u32 = 915_000_000;
    const BITRATE: u32 = 250_000;

    pub fn new(radio_peripherals: RadioPeripherals) -> Self {
        let spi_config = spi::Config::default();
        let spi_driver = Spi::new_blocking(
            radio_peripherals.rf_spi,
            radio_peripherals.rf_sck,
            radio_peripherals.rf_mosi,
            radio_peripherals.rf_miso,
            spi_config,
        );
        let spi_device =
            ExclusiveDevice::new_no_delay(spi_driver, radio_peripherals.rf_cs).unwrap();
        let radio = Rfm69::new(spi_device);

        Self {
            radio,
            reset: radio_peripherals.rf_rst,
            dio_int: ExtiInput::new(
                radio_peripherals.rf_int,
                radio_peripherals.rf_exti,
                Pull::None,
            ),
        }
    }

    pub async fn init(&mut self) -> RadioResult<()> {
        // 7.2.2. Manual Reset Pin
        //
        // RESET should be pulled high for a hundred microseconds, and then
        // released. The user should then wait for 5 ms before using the module.

        self.reset.set_high();
        Timer::after_millis(2).await;
        self.reset.set_low();
        Timer::after_millis(5).await;

        // See if the radio exists
        let version = self.radio.read(registers::Registers::Version)?;
        if version == 0 {
            info!("Radio not found");
            return Err(RadioError::NoRadio);
        }

        debug!("Radio version: {:x}", version);

        use rfm69::registers::Mode;
        use rfm69::registers::*;

        self.radio.mode(Mode::Standby)?;

        // Start TX when first byte reaches FIFO
        self.radio.fifo_mode(FifoMode::NotEmpty)?;

        self.radio
            .continuous_dagc(ContinuousDagc::ImprovedMarginAfcLowBetaOn0)?;

        self.radio
            .dio_mapping(DioMapping {
                pin: DioPin::Dio0,
                dio_type: DioType::Dio01,
                dio_mode: DioMode::Rx,
            })
            .unwrap();

        self.radio.rssi_threshold(220)?;
        self.radio.sync(&[0x2d, 0xd4])?;
        self.radio.packet(PacketConfig {
            format: PacketFormat::Variable(66),
            dc: PacketDc::Whitening,
            filtering: PacketFiltering::None,
            crc: true,
            interpacket_rx_delay: InterPacketRxDelay::Delay2Bits,
            auto_rx_restart: true,
        })?;
        self.radio.modulation(Modulation {
            data_mode: DataMode::Packet,
            modulation_type: ModulationType::Fsk,
            shaping: ModulationShaping::Shaping01,
        })?;
        self.radio.preamble(4)?;
        self.radio.bit_rate(Self::BITRATE)?;
        self.radio.frequency(Self::FREQUENCY)?;
        self.radio.fdev(50_000)?;
        // reg 0x19 RxBw = 0xe0 = 0b11100000
        // -> DccFreq = 7, RxBwMant = 00, RxBwExp = 000
        self.radio.rx_bw(RxBw {
            dcc_cutoff: DccCutoff::Percent0dot125,
            rx_bw: RxBwFsk::Khz500dot0,
        })?;
        self.radio.lna(LnaConfig {
            zin: LnaImpedance::Ohm50,
            gain_select: LnaGain::AgcLoop,
        })?;
        Ok(())
    }

    pub async fn send_packet(&mut self, packet: &Packet) {
        if packet.data.len() > MAX_PAYLOAD_SIZE {
            error!("Data length too long");
            return;
        }

        let mut buf = [0u8; MAX_PAYLOAD_SIZE + 8];
        let wire_data = packet.radio_wire_format(&mut buf);
        debug!("Sending packet: {:x}", wire_data);
        if self.radio.send(wire_data).is_err() {
            error!("Radio send error");
        }
    }

    pub async fn recv_packet(&mut self) -> Packet {
        self.radio.mode(rfm69::registers::Mode::Receiver).unwrap();
        loop {
            self.dio_int.wait_for_rising_edge().await;

            match try_recv(&mut self.radio).await {
                Ok(packet) => return packet,
                Err(RadioError::NoPacketAvailable) => continue,
                Err(e) => {
                    error!("Radio recv error: {:?}", e);
                    continue;
                }
            }
        }

        async fn try_recv(
            radio: &mut Rfm69<ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, NoDelay>>,
        ) -> RadioResult<Packet> {
            // A complete message has been received with good CRC. Must look for
            // PAYLOADREADY, not CRCOK, since only PAYLOADREADY occurs _after_ AES
            // decryption.
            //
            // Note that a bad message can sometimes have a good CRC.

            if radio.read(rfm69::registers::Registers::IrqFlags2)?
                & rfm69::registers::IrqFlags2::PayloadReady
                == 0
            {
                return Err(RadioError::NoPacketAvailable);
            }

            radio.mode(rfm69::registers::Mode::Standby)?;

            let mut buf = [0; 4];
            radio.read_many(rfm69::registers::Registers::Fifo, &mut buf)?;
            debug!("Received buf: {:x}", buf);

            let len = buf[0] as usize;
            let to = buf[1];
            let from = Address(buf[2]);
            let tag = Message::try_from(buf[3]).map_err(|_| RadioError::InvalidPacket)?;

            let mut packet = Packet::new(from, Address(to), tag);

            if len > 0 {
                let _ = packet.data.resize(len - 3, 0);
                radio.read_many(rfm69::registers::Registers::Fifo, &mut packet.data)?;
            }
            debug!("Received data: {:x}", packet.data.as_slice());

            Ok(packet)
        }
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
        let wire_data = packet.serial_wire_format(&mut buf);
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
            let tag = match Message::try_from(tag) {
                Ok(tag) => tag,
                Err(_) => {
                    error!("Invalid tag: {:02x}", tag);
                    continue;
                }
            };

            let data_len = len - 2; // Subtract from and tag
            let mut packet = Packet::new(from, Address(to), tag);

            if data_len > 0 {
                let _ = packet.data.resize(data_len, 0);
                if self
                    .rx
                    .read_exact(&mut packet.data[0..data_len])
                    .await
                    .is_err()
                {
                    continue;
                }
            }

            let crc = self.read_byte().await;
            // TODO: real crc check
            if crc != b'C' {
                error!("CRC error: {:02x}", crc);
                continue;
            }

            // debug!("Received packet: {:?}", packet);

            if to == BROADCAST_ADDRESS.value() || to == self.address.value() {
                return packet;
            }
        }
    }

    async fn read_byte(&mut self) -> u8 {
        let mut buffer = [0; 1];
        if let Err(e) = self.rx.read(&mut buffer).await {
            error!("read_byte error: {:?}", e);
        }
        // debug!("Received: {:02x}", buffer[0]);
        buffer[0]
    }
}
