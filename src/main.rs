#![no_std]
#![no_main]

use crate::blinker::blinker_task;
use crate::debug_port::debug_port_task;
use crate::lights::lights_task;
use crate::panel_bus::panel_bus_task;
use crate::radio::radio_task;
use crate::usb::usb_task;
use comm::{Address, Comm};
use core::{panic::PanicInfo, sync::atomic::AtomicI8};
use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::gpio::Input;
use embassy_time::Timer;
use num_enum::TryFromPrimitive;
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

    let board = board::hookup();

    StatusLEDs::init(board.status_leds);

    let mut app = App::new(board.user_btn, board.pir_1, board.pir_2);
    app.determine_mode().await;

    spawner.must_spawn(blinker_task());
    spawner.must_spawn(debug_port_task(
        board.dbg_usart,
        board.dbg_usart_rx,
        board.dbg_usart_tx,
    ));
    spawner.must_spawn(lights_task(
        board.led_timer,
        board.led_strip.red,
        board.led_strip.green,
        board.led_strip.blue,
    ));

    let comm: &mut dyn Comm;

    if let Ok(radio) = radio::setup_radio(
        board.rf_spi,
        board.rf_sck,
        board.rf_mosi,
        board.rf_miso,
        board.rf_cs,
        board.rf_rst,
    )
    .await
    {
        spawner.must_spawn(radio_task(radio, board.rf_int));
        comm = &mut radio;
    } else {
        defmt::error!("Radio setup failed");

        spawner.must_spawn(panel_bus_task(
            board.panel_bus_usart,
            board.panel_bus_usart_tx,
            board.panel_bus_usart_tx_dma,
            board.panel_bus_usart_rx_dma,
            board.ser_out_en,
        ));
    }
    spawner.must_spawn(usb_task(
        board.usb,
        board.usb_pullup,
        board.usb_dp,
        board.usb_dm,
    ));

    app.run().await;

    loop {
        StatusLEDs::set(3);
        Timer::after_millis(100).await;
        StatusLEDs::reset(3);
        Timer::after_millis(100).await;
    }
}

fn check_boot_status() {
    // Safety: We just booted so there aren't any threads
    unsafe {
        BOOT_COUNT = BOOT_COUNT.wrapping_add(1);
        // Disallow zero so we can use it as a sentinel value
        if BOOT_COUNT == 0 {
            BOOT_COUNT = 1;
        }

        info!("BOOT_MAGIC={:x}", BOOT_MAGIC);
        if BOOT_MAGIC == BOOT_MAGIC_VALUE {
            IS_WARM_BOOT = true;
        } else {
            IS_WARM_BOOT = false;
            core::ptr::write_volatile(&raw mut BOOT_MAGIC, BOOT_MAGIC_VALUE);
        }

        info!("is_warm_boot={}", IS_WARM_BOOT);
    }
}

pub fn is_warm_boot() -> bool {
    // Safety: This is only written once at boot time.
    unsafe { IS_WARM_BOOT }
}

pub fn get_boot_count() -> u8 {
    // Safety: This is only written once at boot time.
    unsafe { BOOT_COUNT }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, TryFromPrimitive)]
#[repr(u8)]
pub enum Mode {
    Master = 1,
    Panel = 2,
    Spy = 3,
}

struct App {
    mode: Mode,
    my_id: Address,
    user_btn: Input<'static>,
    pir_1: Input<'static>,
    pir_2: Input<'static>,
}

impl App {
    fn new(user_btn: Input<'static>, pir_1: Input<'static>, pir_2: Input<'static>) -> Self {
        Self {
            mode: Mode::Panel,
            my_id: Address(flash::get_my_id()),
            user_btn,
            pir_1,
            pir_2,
        }
    }

    async fn run(&mut self) {
        info!(
            "Aunisoma version {} ID={} Mode={}",
            version::VERSION,
            self.my_id.0,
            self.mode as u8
        );

        loop {
            Timer::after_millis(100).await;
        }
    }

    fn user_btn_pressed(&self) -> bool {
        self.user_btn.is_high()
    }

    /// Board 0 is always in Spy mode.
    ///
    /// Boards store their default mode in flash. Uninitialized boards default to
    /// Panel mode. If the button is down at boot, the default mode will be
    /// switched between Master and Panel. The default mode can also be changed
    /// with the 'D' command.

    async fn determine_mode(&mut self) {
        let mut mode = flash::get_default_mode();

        if self.my_id == Address(0) {
            mode = Mode::Spy;
        } else {
            if self.user_btn_pressed() {
                mode = self.toggle_mode().await;
            }
        }

        match mode {
            Mode::Master => {
                StatusLEDs::set(1);
            }
            Mode::Panel => {
                StatusLEDs::set(2);
            }
            Mode::Spy => {
                StatusLEDs::set(1);
                StatusLEDs::set(2);
            }
        }

        self.mode = mode;
    }

    /// Toggle between Master and Panel modes
    ///
    async fn toggle_mode(&mut self) -> Mode {
        let new_mode = match self.mode {
            Mode::Master => Mode::Panel,
            Mode::Panel => Mode::Master,
            _ => self.mode,
        };

        flash::set_default_mode(new_mode);

        info!(
            "Mode is now {}",
            if new_mode == Mode::Master {
                "Master"
            } else {
                "Panel"
            }
        );

        // Blink lights until button is released

        while self.user_btn_pressed() {
            StatusLEDs::set_all(0xF);
            Timer::after_millis(250).await;
            StatusLEDs::set_all(0);
            Timer::after_millis(250).await;
        }

        Timer::after_millis(250).await;

        cortex_m::peripheral::SCB::sys_reset();
    }
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
mod comm;
mod debug_port;
mod flash;
mod lights;
mod logger;
mod master;
mod panel;
mod panel_bus;
mod radio;
mod ring_buffer;
mod status_leds;
mod usb;
mod version;
