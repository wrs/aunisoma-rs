use core::convert::Infallible;

use crate::board::{RadioMiso, RadioMos, RadioSck, RadioSpi};
use embassy_stm32::{
    gpio::{Input, Output},
    mode::Blocking,
    spi::{Config as SpiConfig, Spi},
};
use embassy_time::Timer;
use embedded_hal_bus::spi::{DeviceError, ExclusiveDevice, NoDelay};
use rfm69::registers;
use rfm69::registers::{
    ContinuousDagc, DataMode, DccCutoff, FifoMode, InterPacketRxDelay, LnaConfig, LnaGain,
    LnaImpedance, Mode, Modulation, ModulationShaping, ModulationType, PacketConfig, PacketDc,
    PacketFiltering, PacketFormat, RxBw, RxBwFsk,
};
use rfm69::Rfm69;

const FREQUENCY: u32 = 915_000_000;

#[embassy_executor::task]
pub(crate) async fn radio_task(
    rf_spi: RadioSpi,
    rf_sck: RadioSck,
    rf_mosi: RadioMos,
    rf_miso: RadioMiso,
    rf_cs: Output<'static>,
    rf_int: Input<'static>,
    rf_rst: Output<'static>,
) {
    let mut radio = setup_radio(rf_spi, rf_sck, rf_mosi, rf_miso, rf_cs, rf_int, rf_rst).await.unwrap();
    radio.send(b"Hello, world!").unwrap();
}

type Rfm69Error = rfm69::Error<DeviceError<embassy_stm32::spi::Error, Infallible>>;

async fn setup_radio(
    rf_spi: RadioSpi,
    rf_sck: RadioSck,
    rf_mosi: RadioMos,
    rf_miso: RadioMiso,
    rf_cs: Output<'static>,
    rf_int: Input<'static>,
    mut rf_rst: Output<'static>,
) -> Result<Rfm69<ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, NoDelay>>, Rfm69Error> {
    let spi_config: SpiConfig = Default::default();
    let spi_bus = Spi::new_blocking(rf_spi, rf_sck, rf_mosi, rf_miso, spi_config);
    let spi_device = ExclusiveDevice::new_no_delay(spi_bus, rf_cs).unwrap();
    let mut radio = Rfm69::new(spi_device);

    // 7.2.2. Manual Reset Pin
    //
    // RESET should be pulled high for a hundred microseconds, and then
    // released. The user should then wait for 5 ms before using the module.

    rf_rst.set_high();
    Timer::after_millis(2).await;
    rf_rst.set_low();
    Timer::after_millis(5).await;

    // See if the radio exists
    let version = radio.read(registers::Registers::Version)?;
    if version == 0 {
        defmt::info!("Radio not found");
        return Err(Rfm69Error::Timeout);
    }

    radio
        .dio_mapping(registers::DioMapping {
            pin: registers::DioPin::Dio0,
            dio_type: registers::DioType::Dio01,
            dio_mode: registers::DioMode::Rx,
        })
        .unwrap();
    radio.mode(Mode::Standby)?;
    radio.modulation(Modulation {
        data_mode: DataMode::Packet,
        modulation_type: ModulationType::Fsk,
        shaping: ModulationShaping::Shaping00,
    })?;
    radio.bit_rate(55_555)?;
    radio.frequency(FREQUENCY)?;
    radio.fdev(50_000)?;
    radio.rx_bw(RxBw {
        dcc_cutoff: DccCutoff::Percent4,
        rx_bw: RxBwFsk::Khz125dot0,
    })?;
    radio.preamble(3)?;
    radio.sync(&[0x2d, 0])?;
    radio.packet(PacketConfig {
        format: PacketFormat::Variable(66),
        dc: PacketDc::None,
        filtering: PacketFiltering::None,
        crc: true,
        interpacket_rx_delay: InterPacketRxDelay::Delay2Bits,
        auto_rx_restart: true,
    })?;
    radio.fifo_mode(FifoMode::NotEmpty)?;
    radio.lna(LnaConfig {
        zin: LnaImpedance::Ohm50,
        gain_select: LnaGain::AgcLoop,
    })?;
    radio.rssi_threshold(220)?;
    radio.continuous_dagc(ContinuousDagc::ImprovedMarginAfcLowBetaOn0)?;
    Ok(radio)
}
