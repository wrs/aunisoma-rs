use defmt::info;
use embassy_time::Timer;

use crate::{Mode, board, comm::Address, flash, status_leds::StatusLEDs};

#[unsafe(link_section = ".noinit")]
static mut BOOT_COUNT: u8 = 0;

#[unsafe(link_section = ".noinit")]
static mut BOOT_MAGIC: u32 = 0;
const BOOT_MAGIC_VALUE: u32 = 0x31337cde;

static mut IS_WARM_BOOT: bool = false;

pub fn check_boot_status() {
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
            BOOT_MAGIC = BOOT_MAGIC_VALUE;
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

/// Board 0 is always in Spy mode.
///
/// Boards store their default mode in flash. Uninitialized boards default to
/// Panel mode. If the button is down at boot, the default mode will be
/// switched between Master and Panel. The default mode can also be changed
/// with the 'D' command.

pub fn determine_mode(address: Address) -> Mode {
    if address == Address(0) {
        return Mode::Spy;
    }

    let mode = flash::get_default_mode();

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

    mode
}

/// Toggle between Master and Panel modes
///
pub async fn toggle_mode(mode: Mode) -> ! {
    let new_mode = match mode {
        Mode::Master => Mode::Panel,
        Mode::Panel => Mode::Master,
        _ => mode,
    };

    flash::set_default_mode(new_mode);

    info!("Mode is now {}", new_mode);

    // Blink lights until button is released

    while board::controls().user_btn_is_pressed() {
        StatusLEDs::set_all(0xF);
        Timer::after_millis(250).await;
        StatusLEDs::set_all(0);
        Timer::after_millis(250).await;
    }

    Timer::after_millis(250).await;

    cortex_m::peripheral::SCB::sys_reset();
}
