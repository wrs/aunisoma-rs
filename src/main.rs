#![no_std]
#![no_main]

use crate::blinker::blinker_task;
use crate::debug_port::debug_port_task;
use crate::lights::lights_task;
use crate::panel_bus::panel_bus_task;
use crate::radio::radio_task;
use crate::usb::usb_task;
use core::{panic::PanicInfo, sync::atomic::AtomicI8};
use defmt::println;
use embassy_executor::Spawner;
use embassy_stm32::{gpio::Input, time::Hertz};
use embassy_time::Timer;
// use panic_itm as _;
#[cfg(feature = "use-itm")]
use defmt_itm as _;
#[cfg(feature = "use-rtt")]
use defmt_rtt as _;
use status_leds::StatusLEDs;
// use panic_halt as _;

#[link_section = ".noinit"]
static mut BOOT_COUNT: u8 = 0;

static mut IS_WARM_BOOT: bool = false;
#[link_section = ".noinit"]
static mut BOOT_MAGIC: u32 = 0;
const BOOT_MAGIC_VALUE: u32 = 0xdeadbeef;

// NOTE: Using Executor requires debugging with connect-under-reset.
// See "wfe interfering with RTT and flashing"
// https://github.com/embassy-rs/embassy/issues/1742

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    check_boot_status();

    #[cfg(feature = "use-itm")]
    {
        let cp = cortex_m::Peripherals::take().unwrap();
        defmt_itm::enable(cp.ITM);
    }

    DEFMT_READY.store(1, core::sync::atomic::Ordering::Relaxed);

    defmt::info!("Main task started");

    let board = board::take();

    StatusLEDs::init(board.status_leds);

    spawner.must_spawn(blinker_task());
    spawner.must_spawn(debug_port_task(
        board.dbg_usart,
        board.dbg_usart_rx,
        board.dbg_usart_tx,
    ));
    spawner.must_spawn(panel_bus_task(
        board.panel_bus_usart,
        board.panel_bus_usart_tx,
        board.panel_bus_usart_tx_dma,
        board.panel_bus_usart_rx_dma,
        board.ser_out_en,
    ));
    spawner.must_spawn(lights_task(
        board.led_timer,
        board.led_strip.red,
        board.led_strip.green,
        board.led_strip.blue,
    ));
    spawner.must_spawn(radio_task(
        board.rf_spi,
        board.rf_sck,
        board.rf_mosi,
        board.rf_miso,
        board.rf_cs,
        board.rf_int,
        board.rf_rst,
    ));
    spawner.must_spawn(usb_task(
        board.usb,
        board.usb_pullup,
        board.usb_dp,
        board.usb_dm,
    ));

    let app = App::new(board.pir_1, board.pir_2);
    app.run().await;

    loop {
        StatusLEDs::set(3);
        Timer::after_millis(100).await;
        StatusLEDs::reset(3);
        Timer::after_millis(100).await;
    }
}

fn check_boot_status() {
    // SAFETY: We just booted so there aren't any threads
    unsafe {
        BOOT_COUNT = BOOT_COUNT.wrapping_add(1);
        // Disallow zero so we can use it as a sentinel value
        if BOOT_COUNT == 0 {
            BOOT_COUNT = 1;
        }

        if BOOT_MAGIC == BOOT_MAGIC_VALUE {
            IS_WARM_BOOT = true;
        } else {
            IS_WARM_BOOT = false;
            BOOT_MAGIC = BOOT_MAGIC_VALUE;
        }
    }
}

enum Mode {
    Master,
    Panel,
    Spy,
}

pub struct Address(u8);

struct App {
    mode: Mode,
    my_id: Address,
    pir_1: Input<'static>,
    pir_2: Input<'static>,
}

impl App {
    fn new(pir_1: Input<'static>, pir_2: Input<'static>) -> Self {
        Self {
            mode: Mode::Panel,
            my_id: get_my_id(),
            pir_1,
            pir_2,
        }
    }

    async fn run(&self) {
        loop {
            Timer::after_millis(100).await;
        }
    }
}

fn get_my_id() -> Address {
    let (data0, data1) = flash::get_user_bytes();
    println!("data: {:?}", (data0, data1));
    Address(data0)
}

#[inline(never)]
#[panic_handler] // built-in ("core") attribute
fn core_panic(info: &PanicInfo) -> ! {
    defmt::error!("Panic: {:?}", info);
    loop {}
}

// Work in progress -- not sure how trace was intended to be used

static DEFMT_READY: AtomicI8 = AtomicI8::new(0);

#[no_mangle]
extern "Rust" fn _embassy_trace_task_new(executor_id: u32, task_id: u32) {
    if DEFMT_READY.load(core::sync::atomic::Ordering::Relaxed) != 0 {
        defmt::info!("task_new: executor_id={}, task_id={}", executor_id, task_id);
    }
}
#[no_mangle]
extern "Rust" fn _embassy_trace_task_exec_begin(executor_id: u32, task_id: u32) {
    if DEFMT_READY.load(core::sync::atomic::Ordering::Relaxed) != 0 {
        defmt::info!(
            "task_exec_begin: executor_id={}, task_id={}",
            executor_id,
            task_id
        );
    }
}
#[no_mangle]
extern "Rust" fn _embassy_trace_task_exec_end(executor_id: u32, task_id: u32) {
    if DEFMT_READY.load(core::sync::atomic::Ordering::Relaxed) != 0 {
        defmt::info!(
            "task_exec_end: executor_id={}, task_id={}",
            executor_id,
            task_id
        );
    }
}
#[no_mangle]
extern "Rust" fn _embassy_trace_task_ready_begin(executor_id: u32, task_id: u32) {
    if DEFMT_READY.load(core::sync::atomic::Ordering::Relaxed) != 0 {
        defmt::info!(
            "task_ready_begin: executor_id={}, task_id={}",
            executor_id,
            task_id
        );
    }
}
#[no_mangle]
extern "Rust" fn _embassy_trace_executor_idle(executor_id: u32) {
    if DEFMT_READY.load(core::sync::atomic::Ordering::Relaxed) != 0 {
        defmt::info!("executor_idle: executor_id={}", executor_id);
    }
}

mod blinker;
mod board;
mod debug_port;
mod flash;
mod lights;
mod logger;
mod panel_bus;
mod radio;
mod status_leds;
mod usb;
