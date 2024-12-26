use core::cell::Cell;

use cortex_m::singleton;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, OutputType, Pull, Speed};
use embassy_stm32::peripherals::{SPI1, TIM2, USART1, USART2};
use embassy_stm32::spi::{Config as SpiConfig, Spi};
use embassy_stm32::time::Hertz;
use embassy_stm32::timer::low_level::CountingMode;
use embassy_stm32::timer::simple_pwm::{Ch1, Ch2, Ch4, PwmPin, SimplePwm};
use embassy_stm32::usart::{
    BufferedUart, Config as UsartConfig, HalfDuplexConfig, RxPin, TxPin, Uart,
};
use embassy_stm32::{bind_interrupts, peripherals, usart};
use embassy_time::{Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use rfm69::Rfm69;

use crate::blinker;

/// Maps logical pins to physical pins
///

#[allow(dead_code)]

// ------------------------------------------------------------------------------------------------
// Peripheral assignments for the board

type DbgUsart = USART1;
type DbgUsartRx = peripherals::PA10;
type DbgUsartTx = peripherals::PA9;
type BusUsart = USART2;
type BusUsartTx = peripherals::PA2;
type LedTimer = TIM2;
type RadioSpi = SPI1;

bind_interrupts!(struct Irqs {
        USART1 => usart::BufferedInterruptHandler<USART1>;
        USART2 => usart::InterruptHandler<USART2>;
});

// ------------------------------------------------------------------------------------------------

#[allow(unused_variables)]
#[inline(never)]
pub async fn hookup(spawner: Spawner, p: embassy_stm32::Peripherals) {
    let bus_usart = p.USART2;
    let bus_usart_tx = p.PA2;
    let bus_usart_tx_dma = p.DMA1_CH7;
    let bus_usart_rx_dma = p.DMA1_CH6;
    let dbg_usart = p.USART1;
    let dbg_usart_rx = p.PA10;
    let dbg_usart_tx = p.PA9;
    let led_timer = p.TIM2;
    let led_red = PwmPin::<TIM2, Ch1>::new_ch1(p.PA0, OutputType::PushPull);
    let led_green = PwmPin::<TIM2, Ch2>::new_ch2(p.PA1, OutputType::PushPull);
    #[cfg(feature = "rev-d")]
    let led_blue = PwmPin::<TIM2, Ch4>::new_ch4(p.PA2, OutputType::PushPull);
    #[cfg(feature = "rev-e")]
    let led_blue = PwmPin::<TIM2, Ch4>::new_ch4(p.PA3, OutputType::PushPull);
    let led_status1 = Output::new(p.PB15, Level::Low, Speed::VeryHigh);
    let led_status2 = Output::new(p.PB14, Level::Low, Speed::VeryHigh);
    let led_status3 = Output::new(p.PB13, Level::Low, Speed::VeryHigh);
    let led_status4 = Output::new(p.PB12, Level::Low, Speed::VeryHigh);
    let pir_1 = Input::new(p.PB10, Pull::Up);
    let pir_2 = Input::new(p.PB2, Pull::Up);
    let rf_spi = p.SPI1;
    let rf_sck = p.PA5;
    let rf_miso = p.PA6;
    let rf_mosi = p.PA7;
    let rf_tx_dma = p.DMA1_CH3;
    let rf_rx_dma = p.DMA1_CH2;
    let rf_cs = Output::new(p.PB0, Level::High, Speed::Medium);
    let rf_int = Input::new(p.PB11, Pull::Up);
    let rf_rst = Output::new(p.PB1, Level::High, Speed::Medium);
    let ser_out_en = Output::new(p.PA4, Level::High, Speed::Medium);
    let usb_pullup = Output::new(p.PA15, Level::High, Speed::Low);
    let user_btn = Input::new(p.PA8, Pull::Up);

    unsafe {
        STATUS_LEDS_PTR = singleton!(STATUS_LEDS: StatusLEDs = StatusLEDs {
            leds: [led_status1, led_status2, led_status3, led_status4],
        })
        .unwrap();
    }

    let the_blinker = blinker::Blinker {};
    the_blinker.spawn(spawner);

    let usart_bus = Uart::new_half_duplex(
        bus_usart,
        bus_usart_tx,
        Irqs,
        bus_usart_tx_dma,
        bus_usart_rx_dma,
        UsartConfig::default(),
        HalfDuplexConfig::PushPull,
    )
    .unwrap();

    spawn_dbg(spawner, dbg_usart, dbg_usart_rx, dbg_usart_tx);

    spawner
        .spawn(led_pwm_task(LedPwm::new(
            led_timer, led_red, led_green, led_blue,
        )))
        .unwrap();

    let spi_config: SpiConfig = Default::default();
    let spi_bus = Spi::new_blocking(rf_spi, rf_sck, rf_mosi, rf_miso, spi_config);
    let spi_device = ExclusiveDevice::new_no_delay(spi_bus, rf_cs).unwrap();
    let mut radio = Rfm69::new(spi_device);
    radio.mode(rfm69::registers::Mode::Sleep).unwrap();
    radio.frequency(915_000_000).unwrap();
    // TODO More radio setup


}

struct LedPwm {
    pwm: SimplePwm<'static, LedTimer>,
}

impl LedPwm {
    pub fn new(
        timer: LedTimer,
        led_red: PwmPin<'static, LedTimer, Ch1>,
        led_green: PwmPin<'static, LedTimer, Ch2>,
        led_blue: PwmPin<'static, LedTimer, Ch4>,
    ) -> Self {
        Self {
            pwm: SimplePwm::new(
                timer,
                Some(led_red),
                Some(led_green),
                None,
                Some(led_blue),
                Hertz(1000),
                CountingMode::EdgeAlignedUp,
            ),
        }
    }
}

#[embassy_executor::task]
async fn led_pwm_task(led_pwm: LedPwm) {
    defmt::info!("led_pwm_task started");

    let mut channels = led_pwm.pwm.split();
    channels.ch1.set_duty_cycle_fraction(127, 255);
    channels.ch2.set_duty_cycle_fraction(127, 255);
    channels.ch4.set_duty_cycle_fraction(127, 255);
}

static mut DBG_USART_PTR: Cell<*mut BufferedUart<'static>> = Cell::new(core::ptr::null_mut());

fn spawn_dbg(
    spawner: Spawner,
    usart: DbgUsart,
    rx: impl RxPin<DbgUsart>,
    tx: impl TxPin<DbgUsart>,
) {
    let mut dbg_config = UsartConfig::default();
    dbg_config.baudrate = 230400;

    static mut DBG_TX_BUFFER: [u8; 128] = [0u8; 128];
    static mut DBG_RX_BUFFER: [u8; 128] = [0u8; 128];
    unsafe {
        DBG_USART_PTR.set(singleton!(DBG_USART: BufferedUart =
            BufferedUart::new(usart, Irqs, rx, tx, DBG_TX_BUFFER.as_mut_slice(), DBG_RX_BUFFER.as_mut_slice(), dbg_config).unwrap())
        .unwrap());
    }

    spawner.spawn(dbg_task()).unwrap();
}

#[embassy_executor::task]
async fn dbg_task() {
    defmt::info!("dbg_task started");

    let usart_dbg = unsafe {
        DBG_USART_PTR
            .replace(core::ptr::null_mut())
            .as_mut()
            .unwrap()
    };
    loop {
        let _ = embedded_io_async::Write::write_all(usart_dbg, b"AUNISOMA> ").await;
        Timer::after(Duration::from_millis(100)).await;
    }
}

static mut STATUS_LEDS_PTR: *mut StatusLEDs = core::ptr::null_mut();

pub struct StatusLEDs {
    leds: [Output<'static>; 4],
}

unsafe impl Sync for StatusLEDs {}

impl StatusLEDs {
    pub fn set(which: usize) {
        let leds = unsafe { STATUS_LEDS_PTR.as_mut().unwrap() };
        leds.leds[which].set_level(Level::High);
    }

    pub fn reset(which: usize) {
        let leds = unsafe { STATUS_LEDS_PTR.as_mut().unwrap() };
        leds.leds[which].set_level(Level::Low);
    }
}
