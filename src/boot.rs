use defmt::{debug, info};
use embassy_futures::select::{self, select};
use embassy_time::{Duration, Instant, Timer};

use crate::{
    Mode,
    board::{self, pet_the_watchdog, watchdog_petter},
    comm::{Address, CommMode},
    flash,
    status_leds::StatusLEDs,
};

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

        debug!("BOOT_MAGIC={:x}", BOOT_MAGIC);
        if BOOT_MAGIC == BOOT_MAGIC_VALUE {
            IS_WARM_BOOT = true;
        } else {
            IS_WARM_BOOT = false;
            BOOT_MAGIC = BOOT_MAGIC_VALUE;
        }

        debug!("is_warm_boot={}", IS_WARM_BOOT);
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
///
/// The default comm mode for an uninitialized board is Radio.
///
pub fn determine_mode(address: Address) -> Mode {
    if address == Address(0) {
        return Mode::Spy;
    }

    flash::get_default_mode()
}

/// On-board so-called UX for toggling between modes
///
pub async fn toggle_mode(mode: Mode) -> ! {
    // Status LEDS 0-3 represent the following combinations:

    const SETTINGS: [(Mode, CommMode); 4] = [
        (Mode::Master, CommMode::Radio),
        (Mode::Panel, CommMode::Radio),
        (Mode::Master, CommMode::Serial),
        (Mode::Panel, CommMode::Serial),
    ];

    let user_btn = board::controls().user_btn();

    blink_lights(user_btn).await;

    debug!("Getting comm mode");
    let comm_mode = flash::get_comm_mode();
    let mut index = SETTINGS
        .into_iter()
        .enumerate()
        .find(|(_, modes)| *modes == (mode, comm_mode))
        .map(|(i, _)| i)
        .unwrap_or(0);

    // Short press cycles through the settings, long press
    // writes the current setting to flash and reboots.

    'outer: loop {
        debug!("Index {}", index);
        StatusLEDs::set_all(1 << index);

        while match select::select(watchdog_petter(), user_btn.wait_for_high()).await {
            select::Either::First(_) => true,
            select::Either::Second(_) => false,
        } {}

        let long_press_deadline = Instant::now() + Duration::from_millis(1000);
        while match select::select3(
            watchdog_petter(),
            user_btn.wait_for_low(),
            Timer::at(long_press_deadline),
        )
        .await
        {
            select::Either3::First(_) => true,
            select::Either3::Second(_) => false,
            select::Either3::Third(_) => {
                break 'outer;
            }
        } {}

        index = (index + 1) % SETTINGS.len();
    }

    debug!("Writing mode to flash: {:?}", SETTINGS[index]);

    flash::set_default_mode(SETTINGS[index].0);
    flash::set_comm_mode(SETTINGS[index].1);

    blink_lights(user_btn).await;

    cortex_m::peripheral::SCB::sys_reset();
}

/// Blink lights until button is released
///
async fn blink_lights(
    user_btn: &mut crate::debouncer::Debouncer<embassy_stm32::exti::ExtiInput<'_>>,
) {
    debug!("Blinking lights");
    let mut lights_on = true;
    while match select::select3(
        watchdog_petter(),
        Timer::after_millis(250),
        user_btn.wait_for_low(),
    )
    .await
    {
        select::Either3::First(_) => {
            // Watchdog petted
            true
        }
        select::Either3::Second(_) => {
            StatusLEDs::set_all(if lights_on { 0xF } else { 0 });
            lights_on = !lights_on;
            true
        }
        select::Either3::Third(_) => false,
    } {}
}
