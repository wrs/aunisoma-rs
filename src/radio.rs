use crate::board::{self, RadioMosi};
use crate::comm::{ReceiveCallback, BROADCAST_ADDRESS};
use crate::{
    board::{RadioMiso, RadioSck, RadioSpi},
    comm::{Address, Comm, RxBuffer, MAX_PAYLOAD_SIZE},
};
use core::cell::RefCell;
use core::convert::Infallible;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::{
    gpio::{Output, Pull},
    mode::Blocking,
    spi::{Config as SpiConfig, Spi},
};
use embassy_sync::blocking_mutex;
use embassy_sync::blocking_mutex::raw::{NoopRawMutex, ThreadModeRawMutex};
use embassy_sync::mutex::Mutex;
use embassy_sync::zerocopy_channel::Sender;
use embassy_time::Timer;
use embedded_hal_bus::spi::{DeviceError, ExclusiveDevice, NoDelay};
use heapless::Vec;
use rfm69::registers::{self, Registers};
use rfm69::registers::{
    ContinuousDagc, DataMode, DccCutoff, FifoMode, InterPacketRxDelay, LnaConfig, LnaGain,
    LnaImpedance, Mode, Modulation, ModulationShaping, ModulationType, PacketConfig, PacketDc,
    PacketFiltering, PacketFormat, RxBw, RxBwFsk,
};
use rfm69::Rfm69;

const FREQUENCY: u32 = 915_000_000;
const BITRATE: u32 = 250_000;

pub struct Radio {
    receive_callback: Option<ReceiveCallback>,
    address: Address,
    rfm69: blocking_mutex::Mutex<
        NoopRawMutex,
        RefCell<Rfm69<ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, NoDelay>>>,
    >,
    last_rssi: i8,
}

#[embassy_executor::task]
pub async fn radio_receiver_task(
    radio: &'static Mutex<ThreadModeRawMutex, RefCell<Radio>>,
    rf_int: board::RadioInt,
    rf_exti: board::RadioExti,
    mut radio_rx_sender: Sender<'static, ThreadModeRawMutex, RxBuffer>,
) {
    let mut rf_int_pin = ExtiInput::new(rf_int, rf_exti, Pull::None);
    loop {
        rf_int_pin.wait_for_rising_edge().await;
        let radio = radio.lock().await;
        let radio = radio.borrow();
        let mut rfm69 = radio.rfm69.borrow().borrow_mut();
        let p = radio_rx_sender.send().await;
        if Radio::recv(&mut rfm69, radio.address, &mut p.as_mut_slice()).is_ok() {
            radio_rx_sender.send_done();
        }
        // TODO
        // radio.last_rssi = rfm69.rssi() as i8 / 2;
    }
}

pub enum RadioError {
    Rfm69(rfm69::Error<DeviceError<embassy_stm32::spi::Error, Infallible>>),
    NoRadio,
    Timeout,
    NoPacketAvailable,
    InvalidPacket,
}

impl From<rfm69::Error<DeviceError<embassy_stm32::spi::Error, Infallible>>> for RadioError {
    fn from(e: rfm69::Error<DeviceError<embassy_stm32::spi::Error, Infallible>>) -> Self {
        RadioError::Rfm69(e)
    }
}

type RadioResult<T> = Result<T, RadioError>;

impl Radio {
    pub fn new(
        address: Address,
        receive_callback: Option<ReceiveCallback>,
        rf_spi: RadioSpi,
        rf_sck: RadioSck,
        rf_mosi: RadioMosi,
        rf_miso: RadioMiso,
        rf_cs: Output<'static>,
    ) -> Radio {
        let spi_config: SpiConfig = Default::default();
        let spi_driver = Spi::new_blocking(rf_spi, rf_sck, rf_mosi, rf_miso, spi_config);
        let spi_device = ExclusiveDevice::new_no_delay(spi_driver, rf_cs).unwrap();
        Radio {
            rfm69: blocking_mutex::Mutex::new(RefCell::new(Rfm69::new(spi_device))),
            address,
            receive_callback,
            last_rssi: 0,
        }
    }

    pub async fn init(&mut self, mut rf_rst: Output<'static>) -> RadioResult<()> {
        // 7.2.2. Manual Reset Pin
        //
        // RESET should be pulled high for a hundred microseconds, and then
        // released. The user should then wait for 5 ms before using the module.

        rf_rst.set_high();
        Timer::after_millis(2).await;
        rf_rst.set_low();
        Timer::after_millis(5).await;

        let mut rfm69 = self.rfm69.borrow().borrow_mut();
        // See if the radio exists
        let version = rfm69.read(registers::Registers::Version)?;
        if version == 0 {
            defmt::info!("Radio not found");
            return Err(RadioError::NoRadio);
        }

        rfm69.mode(Mode::Standby)?;

        // Start TX when first byte reaches FIFO
        rfm69.fifo_mode(FifoMode::NotEmpty)?;

        rfm69.continuous_dagc(ContinuousDagc::ImprovedMarginAfcLowBetaOn0)?;

        rfm69
            .dio_mapping(registers::DioMapping {
                pin: registers::DioPin::Dio0,
                dio_type: registers::DioType::Dio01,
                dio_mode: registers::DioMode::Rx,
            })
            .unwrap();

        rfm69.rssi_threshold(220)?;
        rfm69.sync(&[0x2d, 0xd4])?;
        rfm69.packet(PacketConfig {
            format: PacketFormat::Variable(66),
            dc: PacketDc::Whitening,
            filtering: PacketFiltering::None,
            crc: true,
            interpacket_rx_delay: InterPacketRxDelay::Delay2Bits,
            auto_rx_restart: true,
        })?;
        rfm69.modulation(Modulation {
            data_mode: DataMode::Packet,
            modulation_type: ModulationType::Fsk,
            shaping: ModulationShaping::Shaping01,
        })?;
        rfm69.preamble(4)?;
        rfm69.bit_rate(BITRATE)?;
        rfm69.frequency(FREQUENCY)?;
        rfm69.fdev(50_000)?;
        // reg 0x19 RxBw = 0xe0 = 0b11100000
        // -> DccFreq = 7, RxBwMant = 00, RxBwExp = 000
        rfm69.rx_bw(RxBw {
            dcc_cutoff: DccCutoff::Percent0dot125,
            rx_bw: RxBwFsk::Khz500dot0,
        })?;
        assert_eq!(rfm69.read(Registers::RxBw)?, 0xe0);
        rfm69.lna(LnaConfig {
            zin: LnaImpedance::Ohm50,
            gain_select: LnaGain::AgcLoop,
        })?;
        Ok(())
    }

    pub fn send_to(&mut self, to_addr: Address, data: &[u8]) -> Result<(), RadioError> {
        let mut rfm69 = self.rfm69.borrow().borrow_mut();

        let mut packet = Vec::<u8, MAX_PAYLOAD_SIZE>::new();
        packet.push(to_addr.value());
        packet.extend_from_slice(data);
        rfm69.send(&packet)?;
        Ok(())
    }

    pub fn recv(
        rfm69: &mut Rfm69<ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, NoDelay>>,
        my_address: Address,
        into: &mut [u8],
    ) -> Result<(), RadioError> {
        if !rfm69.is_packet_ready()? {
            return Err(RadioError::NoPacketAvailable);
        }

        rfm69.mode(Mode::Standby)?;

        let len = rfm69.read(Registers::Fifo)? as usize;
        if len > MAX_PAYLOAD_SIZE {
            // The chip shouldn't let this happen.
            return Err(RadioError::InvalidPacket);
        }
        if len < 1 {
            return Err(RadioError::InvalidPacket);
        }

        let to_addr_byte = rfm69.read(Registers::Fifo)?;
        let to_addr = Address(to_addr_byte);
        if to_addr != my_address && to_addr != BROADCAST_ADDRESS {
            return Err(RadioError::NoPacketAvailable);
        }
        rfm69.recv(&mut into[..len - 1])?;
        Ok(())
    }

    pub fn last_rssi(&self) -> i8 {
        self.last_rssi
    }
}
