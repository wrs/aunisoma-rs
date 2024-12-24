#![no_std]
#![no_main]

use board::StatusLEDs;
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_itm as _;
use core::fmt::Write;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_stm32::init(Default::default());

    board::hookup(spawner, peripherals).await;

    log!{"Started\n"}
    loop {
        StatusLEDs::set(3);
        Timer::after_millis(2000).await;
        StatusLEDs::reset(3);
        Timer::after_millis(2000).await;
    }
}

// #[inline(never)]
// #[panic_handler]
// fn panic(_info: &PanicInfo) -> ! {
//     log!("PANIC");
//     loop {}
// }

mod board;
mod blinker;
mod logger;
