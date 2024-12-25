#![no_std]
#![no_main]

use board::StatusLEDs;
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_itm as _;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let cp = cortex_m::Peripherals::take().unwrap();
    defmt_itm::enable(cp.ITM);
    defmt::println!("Hello, world!");

    let peripherals = embassy_stm32::init(Default::default());

    board::hookup(spawner, peripherals).await;

    defmt::info!("main task started");
    loop {
        defmt::info!("on");
        StatusLEDs::set(3);
        Timer::after_millis(2000).await;
        defmt::info!("off");
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
