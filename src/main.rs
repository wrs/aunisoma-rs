#![no_std]
#![no_main]

use core::{panic::PanicInfo, sync::atomic::AtomicI8};

use embassy_executor::Spawner;
use embassy_time::Timer;
// use panic_itm as _;
#[cfg(feature = "use-itm")]
use defmt_itm as _;
#[cfg(feature = "use-rtt")]
use defmt_rtt as _;
use status_leds::StatusLEDs;
// use panic_halt as _;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    #[cfg(feature = "use-itm")]
    {
        let cp = cortex_m::Peripherals::take().unwrap();
        defmt_itm::enable(cp.ITM);
    }

    DEFMT_READY.store(1, core::sync::atomic::Ordering::Relaxed);

    defmt::info!("Main task started");

    board::init(spawner).await;

    loop {
        StatusLEDs::set(3);
        Timer::after_millis(100).await;
        StatusLEDs::reset(3);
        Timer::after_millis(100).await;
    }
}

#[inline(never)]
#[panic_handler] // built-in ("core") attribute
fn core_panic(info: &PanicInfo) -> ! {
    defmt::error!("Panic: {}", info);
    loop {}
}

#[defmt::panic_handler]
fn defmt_panic() -> ! {
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
mod lights;
mod logger;
mod panel_bus;
mod radio;
mod status_leds;
mod usb;
