use crate::board::{self, RadioMosi};
use crate::comm::ReceiveCallback;
use crate::ring_buffer::RingBuffer;
use crate::{
    board::{RadioMiso, RadioSck, RadioSpi},
    comm::{Address, Comm, RxBuffer, MAX_PAYLOAD_SIZE},
};
use core::convert::Infallible;
use embassy_stm32::{bind_interrupts, interrupt};
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::Pull;
use embassy_stm32::{
    gpio::{Flex, Output},
    mode::Blocking,
    spi::{Config as SpiConfig, Spi},
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
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

static RADIO_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

struct Radio {
    rfm69: Rfm69<ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, NoDelay>>,
    rx_ring: RingBuffer<RxBuffer, 8>,
    address: Address,
    receive_callback: Option<ReceiveCallback>,
}

#[embassy_executor::task]
pub(crate) async fn radio_task(
    mut radio: Radio,
    rf_int: board::RadioInt,
    rf_exti: board::RadioExti,
) {
    loop {
    }
}

#[interrupt]
fn EXTI15_10() {

}

type Rfm69Error = rfm69::Error<DeviceError<embassy_stm32::spi::Error, Infallible>>;
type RadioResult<T> = Result<T, Rfm69Error>;

impl Radio {
    pub async fn new(
        rf_spi: RadioSpi,
        rf_sck: RadioSck,
        rf_mosi: RadioMosi,
        rf_miso: RadioMiso,
        rf_cs: Output<'static>,
        mut rf_rst: Output<'static>,
        rf_int: Flex<'static>,
    ) -> Result<Radio, Rfm69Error> {
        let spi_config: SpiConfig = Default::default();
        let spi_driver = Spi::new_blocking(rf_spi, rf_sck, rf_mosi, rf_miso, spi_config);
        let spi_device = ExclusiveDevice::new_no_delay(spi_driver, rf_cs).unwrap();

        let mut rfm69 = Rfm69::new(spi_device);

        // 7.2.2. Manual Reset Pin
        //
        // RESET should be pulled high for a hundred microseconds, and then
        // released. The user should then wait for 5 ms before using the module.

        rf_rst.set_high();
        Timer::after_millis(2).await;
        rf_rst.set_low();
        Timer::after_millis(5).await;

        // See if the radio exists
        let version = rfm69.read(registers::Registers::Version)?;
        if version == 0 {
            defmt::info!("Radio not found");
            return Err(Rfm69Error::Timeout);
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

        Ok(Radio {
            rfm69,
            rx_ring: RingBuffer::new(),
            address: Address(0),
            receive_callback: None,
        })
    }
}

impl Comm for Radio {
    fn name(&self) -> &str {
        "Radio"
    }

    fn send_to(&mut self, to_addr: Address, data: &[u8]) -> bool {
        let mut packet = Vec::<u8, MAX_PAYLOAD_SIZE>::new();
        packet.push(to_addr.value());
        packet.extend_from_slice(data);
        self.rfm69.send(&packet).unwrap();
        true
    }

    fn available(&self) -> bool {
        !self.rx_ring.is_empty()
    }

    fn recv(&mut self) -> Option<&RxBuffer> {
        self.rx_ring.next_read()
    }

    fn set_receive_callback(&mut self, callback: Option<ReceiveCallback>) {
        self.receive_callback = callback;
    }

    fn set_address(&mut self, address: Address) {
        self.address = address;
    }

    fn set_default_to_rx_mode(&mut self) {
        todo!()
    }

    fn set_spy_mode(&mut self, is_spy_mode: bool) {
        todo!()
    }

    fn last_rssi(&self) -> i8 {
        self.rfm69.rssi() as i8 / 2
    }
}
