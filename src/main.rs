#![no_std]
#![no_main]

use board::StatusLEDs;
use embassy_executor::Spawner;
use embassy_time::Timer;
// use panic_halt as _;
use core::panic::PanicInfo;
use core::fmt::Write;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_stm32::init(Default::default());

    board::Board::hookup(spawner, peripherals).await;

    log!{"Started\n"}
    loop {
        StatusLEDs::set(4);
        Timer::after_millis(1000).await;
        StatusLEDs::reset(4);
        Timer::after_millis(1000).await;
    }
}

#[inline(never)]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    log!("PANIC\n");
    loop {}
}

mod board;
mod blinker;
mod logger;
