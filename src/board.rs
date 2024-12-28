use crate::blinker::blinker_task;
use crate::debug_port::debug_port_task;
use crate::lights::lights_task;
use crate::panel_bus::panel_bus_task;
use crate::radio::radio_task;
use crate::status_leds::StatusLEDs;
use crate::usb::usb_task;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, OutputType, Pull, Speed};
use embassy_stm32::peripherals;
use embassy_stm32::peripherals::{SPI1, TIM2, USART1, USART2};
use embassy_stm32::time::Hertz;
use embassy_stm32::timer::simple_pwm::{Ch1, Ch2, Ch4, PwmPin};

#[allow(dead_code)]

pub type DbgUsart = USART1;
pub type DbgUsartRx = peripherals::PA10;
pub type DbgUsartTx = peripherals::PA9;
pub type PanelBusUsart = USART2;
pub type PanelBusUsartTx = peripherals::PA2;
pub type PanelBusUsartTxDma = peripherals::DMA1_CH7;
pub type PanelBusUsartRxDma = peripherals::DMA1_CH6;
pub type LedTimer = TIM2;
pub type RadioSpi = SPI1;
pub type RadioSck = peripherals::PA5;
pub type RadioMiso = peripherals::PA6;
pub type RadioMos = peripherals::PA7;
pub type UsbDp = peripherals::PA12;
pub type UsbDm = peripherals::PA11;

// ------------------------------------------------------------------------------------------------

#[allow(unused_variables)]
#[inline(never)]
pub async fn init(spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hse = Some(Hse {
            freq: Hertz(16_000_000),
            mode: HseMode::Oscillator,
        });
        config.rcc.pll = Some(Pll {
            src: PllSource::HSE,
            prediv: PllPreDiv::DIV2,
            mul: PllMul::MUL9,
        });
        config.rcc.sys = Sysclk::PLL1_P;
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV2;
        config.rcc.apb2_pre = APBPrescaler::DIV1;
    }

    let p = embassy_stm32::init(config);

    // Unmap the JTAG pins so we can use PA15 as a GPIO.
    embassy_stm32::pac::AFIO
        .mapr()
        .modify(|w| w.set_swj_cfg(0b010));

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
    let rf_rst = Output::new(p.PB1, Level::Low, Speed::Medium);
    let ser_out_en = Output::new(p.PA4, Level::High, Speed::Medium);
    let usb_dp = p.PA12;
    let usb_dm = p.PA11;
    let usb_pullup = Output::new(p.PA15, Level::High, Speed::Low);
    let user_btn = Input::new(p.PA8, Pull::Up);

    StatusLEDs::init([led_status1, led_status2, led_status3, led_status4]);

    spawner.must_spawn(blinker_task());
    spawner.must_spawn(debug_port_task(dbg_usart, dbg_usart_rx, dbg_usart_tx));
    spawner.must_spawn(panel_bus_task(
        bus_usart,
        bus_usart_tx,
        bus_usart_tx_dma,
        bus_usart_rx_dma,
    ));
    spawner.must_spawn(lights_task(led_timer, led_red, led_green, led_blue));
    spawner.must_spawn(radio_task(
        rf_spi, rf_sck, rf_mosi, rf_miso, rf_cs, rf_int, rf_rst,
    ));
    spawner.must_spawn(usb_task(p.USB, usb_pullup, usb_dp, usb_dm));
}
