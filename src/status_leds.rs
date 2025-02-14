use core::cell::RefCell;

use cortex_m::interrupt::{Mutex, free};
use embassy_stm32::gpio::Output;

pub struct StatusLEDs {
    pub leds: [Output<'static>; 4],
}

static STATUS_LEDS: Mutex<RefCell<Option<StatusLEDs>>> = Mutex::new(RefCell::new(None));

impl StatusLEDs {
    pub fn init(mut leds: [Output<'static>; 4]) {
        for led in leds.iter_mut() {
            led.set_low();
        }
        free(|cs| {
            let status = STATUS_LEDS.borrow(cs);
            *status.borrow_mut() = Some(StatusLEDs { leds });
        });
    }

    #[inline(never)]
    pub fn set(which: usize) {
        free(|cs| {
            let mut status_leds = STATUS_LEDS.borrow(cs).borrow_mut();
            let leds = status_leds.as_mut().unwrap();
            leds.leds[which].set_high();
        });
    }

    #[inline(never)]
    pub fn reset(which: usize) {
        free(|cs| {
            let mut status_leds = STATUS_LEDS.borrow(cs).borrow_mut();
            let leds = status_leds.as_mut().unwrap();
            leds.leds[which].set_low();
        });
    }

    pub fn set_all(value: u8) {
        free(|cs| {
            let mut status_leds = STATUS_LEDS.borrow(cs).borrow_mut();
            let leds = status_leds.as_mut().unwrap();
            for i in 0..4 {
                if value & (1 << i) != 0 {
                    leds.leds[i].set_high();
                } else {
                    leds.leds[i].set_low();
                }
            }
        });
    }
}
