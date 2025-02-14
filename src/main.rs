#![no_std]
#![no_main]

extern crate alloc;

use comm::Address;
use defmt::{info, Format};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_time::Timer;
use embedded_alloc::LlffHeap as Heap;
use num_enum::TryFromPrimitive;
use status_leds::StatusLEDs;
// use panic_halt as _;

#[global_allocator]
static HEAP: Heap = Heap::empty();

// NOTE: Using Executor requires debugging with connect-under-reset.
// See "wfe interfering with RTT and flashing"
// https://github.com/embassy-rs/embassy/issues/1742

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    defmt::info!("\n-----\nMain task started\n-----");

    // Initialize the heap
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 1024;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        #[allow(static_mut_refs)]
        unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) };
    }

    boot::check_boot_status();

    let board = board::hookup();

    StatusLEDs::init(board.status_leds);

    spawner.must_spawn(blinker::blinker_task());

    let address = Address(0);
    let mode = boot::determine_mode(address);
    if board::controls().user_btn_is_pressed() {
        boot::toggle_mode(mode).await;
    }

    info!(
        "Aunisoma version {} ID={} Mode={}",
        version::VERSION,
        address.0,
        mode as u8
    );

    match mode {
        Mode::Master => {
            // let mut master = Master::new(self.my_id, comm);
            // master.run(serial).await;
        }
        Mode::Panel => {
            // let mut panel = Panel::new(app);
            // panel.run(serial, &mut comm).await;
        }
        Mode::Spy => loop {
            Timer::after_millis(100).await;
        },
    }

    loop {
        StatusLEDs::set(3);
        Timer::after_millis(100).await;
        StatusLEDs::reset(3);
        Timer::after_millis(100).await;
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Format, TryFromPrimitive)]
#[repr(u8)]
pub enum Mode {
    Master = 1,
    Panel = 2,
    Spy = 3,
}

#[inline(never)]
#[panic_handler] // built-in ("core") attribute
fn core_panic(info: &core::panic::PanicInfo<'_>) -> ! {
    defmt::error!("Panic: {:?}", info);
    loop {}
}

mod blinker;
mod board;
mod boot;
mod comm;
mod flash;
mod status_leds;
mod version;
