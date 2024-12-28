use core::cell::RefCell;
use embassy_stm32::gpio::Output;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::blocking_mutex::Mutex;

pub struct StatusLEDs {
    pub(crate) leds: [Output<'static>; 4],
}

pub(crate) static STATUS_LEDS: Mutex<ThreadModeRawMutex, RefCell<Option<StatusLEDs>>> =
    Mutex::new(RefCell::new(None));

impl StatusLEDs {
    pub fn init(leds: [Output<'static>; 4]) {
        STATUS_LEDS.lock(|cell| cell.replace(Some(StatusLEDs { leds })));
    }

    #[inline(never)]
    pub(crate) fn with_leds(f: impl FnOnce(&mut StatusLEDs)) {
        STATUS_LEDS.lock(|cell| {
            let mut leds = cell.borrow_mut();
            match &mut *leds {
                Some(leds) => f(leds),
                None => (),
            }
        });
    }

    #[inline(never)]
    pub fn set(which: usize) {
        StatusLEDs::with_leds(|leds| leds.leds[which].set_high());
    }

    #[inline(never)]
    pub fn reset(which: usize) {
        StatusLEDs::with_leds(|leds| leds.leds[which].set_low());
    }
}
