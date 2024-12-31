use embassy_stm32::gpio::{Flex, Input, Level, Output, OutputType, Pull, Speed};
use embassy_stm32::peripherals;
use embassy_stm32::peripherals::{SPI1, TIM2, USART1, USART2};
use embassy_stm32::time::Hertz;
use embassy_stm32::timer::simple_pwm::{self, PwmPin};

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

pub struct LedStrip {
    pub red: PwmPin<'static, TIM2, simple_pwm::Ch1>,
    pub green: PwmPin<'static, TIM2, simple_pwm::Ch2>,
    pub blue: PwmPin<'static, TIM2, simple_pwm::Ch4>,
}

pub struct Board {
    pub dbg_usart: USART1,
    pub dbg_usart_rx: peripherals::PA10,
    pub dbg_usart_tx: peripherals::PA9,
    pub panel_bus_usart: USART2,
    pub panel_bus_usart_tx: peripherals::PA2,
    pub panel_bus_usart_tx_dma: peripherals::DMA1_CH7,
    pub panel_bus_usart_rx_dma: peripherals::DMA1_CH6,
    pub led_strip: LedStrip,
    pub status_leds: [Output<'static>; 4],
    pub led_timer: TIM2,
    pub pir_1: Input<'static>,
    pub pir_2: Input<'static>,
    pub rf_cs: Output<'static>,
    pub rf_int: Flex<'static>,
    pub rf_rst: Output<'static>,
    pub rf_spi: SPI1,
    pub rf_sck: peripherals::PA5,
    pub rf_miso: peripherals::PA6,
    pub rf_mosi: peripherals::PA7,
    pub ser_out_en: Output<'static>,
    pub usb: peripherals::USB,
    pub usb_dp: peripherals::PA12,
    pub usb_dm: peripherals::PA11,
    pub usb_pullup: Output<'static>,
    pub user_btn: Input<'static>,
}

#[allow(unused_variables)]
#[inline(never)]
pub fn take() -> Board {
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

    #[cfg(feature = "rev-d")]
    let led_blue = PwmPin::<TIM2, simple_pwm::Ch3>::new_ch3(p.PA2, OutputType::PushPull);
    #[cfg(feature = "rev-e")]
    let led_blue = PwmPin::<TIM2, simple_pwm::Ch4>::new_ch4(p.PA3, OutputType::PushPull);

    Board {
        dbg_usart: p.USART1,
        dbg_usart_rx: p.PA10,
        dbg_usart_tx: p.PA9,
        panel_bus_usart: p.USART2,
        panel_bus_usart_tx: p.PA2,
        panel_bus_usart_tx_dma: p.DMA1_CH7,
        panel_bus_usart_rx_dma: p.DMA1_CH6,
        led_strip: LedStrip {
            red: PwmPin::<TIM2, simple_pwm::Ch1>::new_ch1(p.PA0, OutputType::PushPull),
            green: PwmPin::<TIM2, simple_pwm::Ch2>::new_ch2(p.PA1, OutputType::PushPull),
            blue: led_blue,
        },
        status_leds: [
            Output::new(p.PB15, Level::High, Speed::VeryHigh),
            Output::new(p.PB14, Level::High, Speed::VeryHigh),
            Output::new(p.PB13, Level::High, Speed::VeryHigh),
            Output::new(p.PB12, Level::High, Speed::VeryHigh),
        ],
        led_timer: p.TIM2,
        pir_1: Input::new(p.PB10, Pull::Up),
        pir_2: Input::new(p.PB2, Pull::Up),
        rf_cs: Output::new(p.PB0, Level::High, Speed::VeryHigh),
        rf_int: Flex::new(p.PB11),
        rf_rst: Output::new(p.PB1, Level::High, Speed::VeryHigh),
        rf_spi: p.SPI1,
        rf_sck: p.PA5,
        rf_miso: p.PA6,
        rf_mosi: p.PA7,
        ser_out_en: Output::new(p.PA4, Level::High, Speed::VeryHigh),
        usb: p.USB,
        usb_dp: p.PA12,
        usb_dm: p.PA11,
        usb_pullup: Output::new(p.PA15, Level::High, Speed::VeryHigh),
        user_btn: Input::new(p.PA8, Pull::Up),
    }
}
