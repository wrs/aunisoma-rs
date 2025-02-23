use core::sync::atomic::{AtomicU32, Ordering};
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Input, Level, Output, OutputType, Pull, Speed};
use embassy_stm32::peripherals::{self, IWDG};
use embassy_stm32::peripherals::{SPI1, TIM2, USART1, USART2};
use embassy_stm32::time::Hertz;
use embassy_stm32::timer::low_level::CountingMode;
use embassy_stm32::timer::simple_pwm::{self, PwmPin, SimplePwm, SimplePwmChannel};
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_time::{Duration, Instant, Timer};

use crate::debouncer::Debouncer;

pub type DbgUsart = USART1;
pub type DbgUsartRx = peripherals::PA10;
pub type DbgUsartTx = peripherals::PA9;
pub type PanelBusUsart = USART2;
pub type PanelBusUsartTx = peripherals::PA2;
pub type LedTimer = TIM2;
pub type RadioSpi = SPI1;
pub type RadioSck = peripherals::PA5;
pub type RadioMiso = peripherals::PA6;
pub type RadioMosi = peripherals::PA7;
pub type RadioInt = peripherals::PB11;
pub type RadioExti = peripherals::EXTI11; // really EXTI15_10
pub type UsbDp = peripherals::PA12;
pub type UsbDm = peripherals::PA11;

pub struct LedStrip {
    pub red_pwm: SimplePwmChannel<'static, LedTimer>,
    pub green_pwm: SimplePwmChannel<'static, LedTimer>,
    pub blue_pwm: SimplePwmChannel<'static, LedTimer>,
}

impl LedStrip {
    pub fn set_colors(&mut self, red: u8, green: u8, blue: u8) {
        self.red_pwm.set_duty_cycle_fraction(255 - red as u16, 255);
        self.green_pwm
            .set_duty_cycle_fraction(255 - green as u16, 255);
        self.blue_pwm
            .set_duty_cycle_fraction(255 - blue as u16, 255);
    }
}

pub struct CmdPortPeripherals {
    pub cmd_usart: DbgUsart,
    pub cmd_usart_rx: DbgUsartRx,
    pub cmd_usart_tx: DbgUsartTx,
}

pub struct PanelBusPeripherals {
    pub panel_bus_usart: PanelBusUsart,
    pub panel_bus_usart_tx: PanelBusUsartTx,
    pub ser_out_en: Output<'static>,
}

pub struct RadioPeripherals {
    pub rf_cs: Output<'static>,
    pub rf_int: RadioInt,
    pub rf_exti: RadioExti,
    pub rf_rst: Output<'static>,
    pub rf_spi: RadioSpi,
    pub rf_sck: RadioSck,
    pub rf_miso: RadioMiso,
    pub rf_mosi: RadioMosi,
}

pub struct UsbPeripherals {
    pub usb: peripherals::USB,
    pub usb_dp: UsbDp,
    pub usb_dm: UsbDm,
    pub usb_pullup: Output<'static>,
}

pub struct Pirs {
    pub pir_1: Input<'static>,
    pub pir_2: Input<'static>,
}

pub struct Board {
    pub cmd_port: CmdPortPeripherals,
    pub panel_bus: PanelBusPeripherals,
    pub radio: RadioPeripherals,
    pub usb: UsbPeripherals,
    pub led_strip: LedStrip,
    pub status_leds: [Output<'static>; 4],
    pub pirs: Pirs,
}

#[allow(unused_variables)]
#[inline(never)]
pub fn hookup() -> Board {
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

    unsafe {
        let watchdog = IndependentWatchdog::new(p.IWDG, 1_000_000);
        WATCHDOG = Some(watchdog);
    }

    // Unmap the JTAG pins so we can use PA15 as a GPIO.
    embassy_stm32::pac::AFIO
        .mapr()
        .modify(|w| w.set_swj_cfg(0b010));

    let led_red = PwmPin::<TIM2, simple_pwm::Ch1>::new_ch1(p.PA0, OutputType::PushPull);
    let led_green = PwmPin::<TIM2, simple_pwm::Ch2>::new_ch2(p.PA1, OutputType::PushPull);
    #[cfg(feature = "rev-d")]
    let led_blue = PwmPin::<TIM2, simple_pwm::Ch3>::new_ch3(p.PA2, OutputType::PushPull);
    #[cfg(feature = "rev-e")]
    let led_blue = PwmPin::<TIM2, simple_pwm::Ch4>::new_ch4(p.PA3, OutputType::PushPull);

    let mut pwm = SimplePwm::new(
        p.TIM2,
        Some(led_red),
        Some(led_green),
        None,
        Some(led_blue),
        Hertz(1000),
        CountingMode::EdgeAlignedUp,
    )
    .split();
    pwm.ch1.enable();
    pwm.ch1.set_duty_cycle_fraction(255, 255);
    pwm.ch2.enable();
    pwm.ch2.set_duty_cycle_fraction(255, 255);
    #[cfg(feature = "rev-d")]
    {
        pwm.ch3.enable();
        pwm.ch3.set_duty_cycle_fraction(255, 255);
    }
    #[cfg(feature = "rev-e")]
    {
        pwm.ch4.enable();
        pwm.ch4.set_duty_cycle_fraction(255, 255);
    }

    unsafe {
        CONTROLS = Some(Controls::new(ExtiInput::new(p.PA8, p.EXTI8, Pull::Down)));
    }

    Board {
        cmd_port: CmdPortPeripherals {
            cmd_usart: p.USART1,
            cmd_usart_rx: p.PA10,
            cmd_usart_tx: p.PA9,
        },
        panel_bus: PanelBusPeripherals {
            panel_bus_usart: p.USART2,
            panel_bus_usart_tx: p.PA2,
            ser_out_en: Output::new(p.PA4, Level::High, Speed::VeryHigh),
        },
        radio: RadioPeripherals {
            rf_cs: Output::new(p.PB0, Level::High, Speed::VeryHigh),
            rf_int: p.PB11,
            rf_exti: p.EXTI11,
            rf_rst: Output::new(p.PB1, Level::High, Speed::VeryHigh),
            rf_spi: p.SPI1,
            rf_sck: p.PA5,
            rf_miso: p.PA6,
            rf_mosi: p.PA7,
        },
        usb: UsbPeripherals {
            usb: p.USB,
            usb_dp: p.PA12,
            usb_dm: p.PA11,
            usb_pullup: Output::new(p.PA15, Level::High, Speed::VeryHigh),
        },
        led_strip: LedStrip {
            red_pwm: pwm.ch1,
            green_pwm: pwm.ch2,
            blue_pwm: pwm.ch4,
        },
        status_leds: [
            Output::new(p.PB15, Level::High, Speed::VeryHigh),
            Output::new(p.PB14, Level::High, Speed::VeryHigh),
            Output::new(p.PB13, Level::High, Speed::VeryHigh),
            Output::new(p.PB12, Level::High, Speed::VeryHigh),
        ],
        pirs: Pirs {
            pir_1: Input::new(p.PB10, Pull::None),
            pir_2: Input::new(p.PB2, Pull::None),
        },
    }
}

static mut WATCHDOG: Option<IndependentWatchdog<'static, IWDG>> = None;

pub fn unleash_the_watchdog() {
    unsafe {
        #[allow(static_mut_refs)]
        WATCHDOG.as_mut().unwrap().unleash();
    }
}

pub fn pet_the_watchdog() {
    // debug!("Petting watchdog");
    unsafe {
        #[allow(static_mut_refs)]
        WATCHDOG.as_mut().unwrap().pet();
    }
}

pub async fn watchdog_petter() {
    const WATCHDOG_INTERVAL_MS: u64 = 500;

    // Scale to make it fit in u32 but still last a long time
    const DEADLINE_SCALE: u64 = 100;
    static NEXT_DEADLINE: AtomicU32 = AtomicU32::new(0);

    let mut deadline_in_ms: u64 = NEXT_DEADLINE.load(Ordering::Relaxed) as u64 * DEADLINE_SCALE;

    if deadline_in_ms == 0 {
        // New interval
        deadline_in_ms = Instant::now().as_millis() + WATCHDOG_INTERVAL_MS;
        NEXT_DEADLINE.store((deadline_in_ms / DEADLINE_SCALE) as u32, Ordering::Release);
        // debug!("New watchdog deadline: {} ms", deadline_in_ms);
    }

    Timer::at(Instant::from_millis(deadline_in_ms)).await;

    pet_the_watchdog();

    NEXT_DEADLINE.store(0, Ordering::Release);
}

pub struct Controls {
    pub user_btn: Debouncer<ExtiInput<'static>>,
}

impl Controls {
    pub fn new(input: ExtiInput<'static>) -> Self {
        Self {
            user_btn: Debouncer::new(input, Duration::from_millis(10)),
        }
    }

    pub fn user_btn(&mut self) -> &mut Debouncer<ExtiInput<'static>> {
        &mut self.user_btn
    }
}

static mut CONTROLS: Option<Controls> = None;

pub fn controls() -> &'static mut Controls {
    #[allow(static_mut_refs)]
    unsafe {
        CONTROLS.as_mut().unwrap()
    }
}
