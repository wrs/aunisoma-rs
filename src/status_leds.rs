use alloc::boxed::Box;
use embassy_stm32::gpio::Output;

pub struct StatusLEDs {
    pub leds: [Output<'static>; 4],
}

static mut STATUS_LEDS: *mut StatusLEDs = core::ptr::null_mut();

impl StatusLEDs {
    pub fn init(mut leds: [Output<'static>; 4]) {
        for led in leds.iter_mut() {
            led.set_low();
        }

        unsafe {
            STATUS_LEDS = Box::leak(Box::new(StatusLEDs { leds }));
        }
    }

    #[inline(never)]
    pub fn set(which: usize) {
        unsafe {
            (*STATUS_LEDS).leds[which].set_high();
        }
    }

    #[inline(never)]
    pub fn reset(which: usize) {
        unsafe {
            (*STATUS_LEDS).leds[which].set_low();
        }
    }

    pub fn set_all(value: u8) {
        unsafe {
            for i in 0..4 {
                if value & (1 << i) != 0 {
                    (*STATUS_LEDS).leds[i].set_high();
                } else {
                    (*STATUS_LEDS).leds[i].set_low();
                }
            }
        }
    }

    #[inline(always)]
    pub fn set_fast(which: usize) {
        if which < 4 {
            embassy_stm32::pac::GPIOB
                .bsrr()
                .write(|w| w.set_bs(15 - which, true));
        }
    }

    #[inline(always)]
    pub fn reset_fast(which: usize) {
        if which < 4 {
            embassy_stm32::pac::GPIOB
                .bsrr()
                .write(|w| w.set_br(15 - which, true));
        }
    }
}
