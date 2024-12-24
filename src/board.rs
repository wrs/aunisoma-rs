use core::cell::Cell;

use cortex_m::singleton;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::interrupt::typelevel::Binding;
use embassy_stm32::usart::{
    BufferedInterruptHandler, BufferedUart, Config as UsartConfig, HalfDuplexConfig, Instance,
    RxPin, TxPin, Uart,
};
use embassy_stm32::{bind_interrupts, peripherals, usart};
use embedded_io_async::Write;

use crate::blinker;

/// Maps logical pins to physical pins
///

#[allow(dead_code)]
pub struct Board {}

bind_interrupts!(struct Irqs {
        USART1 => usart::BufferedInterruptHandler<peripherals::USART1>;
        USART2 => usart::InterruptHandler<peripherals::USART2>;
});

type DbgUsart = peripherals::USART1;
type BusUsart = peripherals::USART2;

impl Board {
    #[allow(unused_variables)]
    pub async fn hookup(spawner: Spawner, p: embassy_stm32::Peripherals) {
        let led_blue = Output::new(p.PA0, Level::Low, Speed::Low);
        let led_green = Output::new(p.PA1, Level::Low, Speed::Low);
        let led_red = Output::new(p.PA3, Level::Low, Speed::Low);
        let led_status1 = Output::new(p.PB15, Level::Low, Speed::VeryHigh);
        let led_status2 = Output::new(p.PB14, Level::Low, Speed::VeryHigh);
        let led_status3 = Output::new(p.PB13, Level::Low, Speed::VeryHigh);
        let led_status4 = Output::new(p.PB12, Level::Low, Speed::VeryHigh);
        let pir_1 = Input::new(p.PB10, Pull::Up);
        let pir_2 = Input::new(p.PB2, Pull::Up);
        let rf_cs = Output::new(p.PB0, Level::High, Speed::Medium);
        let rf_int = Input::new(p.PB11, Pull::Up);
        let rf_rst = Output::new(p.PB1, Level::High, Speed::Medium);
        let ser_out_en = Output::new(p.PA4, Level::High, Speed::Medium);
        let usb_pullup = Output::new(p.PA15, Level::High, Speed::Low);
        let user_btn = Input::new(p.PA8, Pull::Up);

        unsafe {
            STATUS_LEDS_PTR = singleton!(STATUS_LEDS: StatusLEDs = StatusLEDs {
                leds: [led_status3, led_status4],
            })
            .unwrap();
        }

        let the_blinker = blinker::Blinker {
            led: led_status1,
            led2: led_status2,
        };
        the_blinker.spawn(spawner);

        let mut usart_bus = Uart::new_half_duplex(
            p.USART2,
            p.PA2,
            Irqs,
            p.DMA1_CH7,
            p.DMA1_CH6,
            UsartConfig::default(),
            HalfDuplexConfig::PushPull,
        )
        .unwrap();

        spawn_dbg(spawner, p.USART1, p.PA10, p.PA9);

        loop {
            let _ = usart_bus.write_all(b"AUNISOMA> ").await;
        }
    }
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
    let usart_dbg = unsafe { DBG_USART_PTR.replace(core::ptr::null_mut()).as_mut().unwrap() };
    loop {
        let _ = usart_dbg.write_all(b"AUNISOMA> ").await;
    }
}

static mut STATUS_LEDS_PTR: *mut StatusLEDs = core::ptr::null_mut();

pub struct StatusLEDs {
    leds: [Output<'static>; 2],
}

unsafe impl Sync for StatusLEDs {}

impl StatusLEDs {
    pub fn set(which: usize) {
        let leds = unsafe { STATUS_LEDS_PTR.as_mut().unwrap() };
        if which < leds.leds.len() {
            leds.leds[which].set_level(Level::High);
        }
    }

    pub fn reset(which: usize) {
        let leds = unsafe { STATUS_LEDS_PTR.as_mut().unwrap() };
        if which < leds.leds.len() {
            leds.leds[which].set_level(Level::Low);
        }
    }
}

// Set GPIOB directly to control status LEDs as fast as possible.
// Status LEDs are assumed to be on PB15-12, active high.

pub fn set_status(which: u8) {
    if which < 4 {
        embassy_stm32::pac::GPIOB
            .bsrr()
            .write(|w| w.set_bs(15 - which as usize, true));
    }
}

pub fn reset_status(which: u8) {
    if which < 4 {
        embassy_stm32::pac::GPIOB
            .bsrr()
            .write(|w| w.set_br(15 - which as usize, true));
    }
}
