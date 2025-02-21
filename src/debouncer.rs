#![allow(dead_code)]

use embassy_time::{Duration, Timer};
use embedded_hal::digital::v2::InputPin;
use embedded_hal_async::digital::Wait;

pub struct Debouncer<T> {
    input: T,
    debounce_high_time: Duration,
    debounce_low_time: Duration,
}

impl<T> Debouncer<T> {
    pub fn new(input: T, debounce_time: Duration) -> Self {
        Self {
            input,
            debounce_high_time: debounce_time,
            debounce_low_time: debounce_time,
        }
    }

    pub fn new_asymmetric(
        input: T,
        debounce_high_time: Duration,
        debounce_low_time: Duration,
    ) -> Self {
        Self {
            input,
            debounce_high_time,
            debounce_low_time,
        }
    }
}

// In embedded_hal::digital::v2, ErrorType is hidden, so I'm not sure how to
// implement the actual Wait trait here.

impl<T: Wait + InputPin> Debouncer<T> {
    pub async fn wait_for_high(&mut self) {
        if self.input.is_low().unwrap_or(false) {
            loop {
                let _ = self.input.wait_for_rising_edge().await;

                Timer::after(self.debounce_high_time).await;

                if self.input.is_high().unwrap_or(false) {
                    break;
                }
            }
        }
    }

    pub async fn wait_for_low(&mut self) {
        if self.input.is_high().unwrap_or(false) {
            loop {
                let _ = self.input.wait_for_falling_edge().await;

                Timer::after(self.debounce_low_time).await;

                if self.input.is_low().unwrap_or(false) {
                    break;
                }
            }
        }
    }

    pub async fn wait_for_rising_edge(&mut self) {
        loop {
            let _ = self.input.wait_for_rising_edge().await;

            Timer::after(self.debounce_high_time).await;

            if self.input.is_high().unwrap_or(false) {
                break;
            }
        }
    }

    pub async fn wait_for_falling_edge(&mut self) {
        loop {
            let _ = self.input.wait_for_falling_edge().await;

            Timer::after(self.debounce_low_time).await;

            if self.input.is_low().unwrap_or(false) {
                break;
            }
        }
    }

    pub async fn wait_for_any_edge(&mut self) {
        if self.input.is_low().unwrap_or(false) {
            self.wait_for_rising_edge().await;
        } else {
            self.wait_for_falling_edge().await;
        }
    }

    pub fn is_high(&self) -> bool {
        self.input.is_high().unwrap_or(false)
    }

    pub fn is_low(&self) -> bool {
        self.input.is_low().unwrap_or(false)
    }
}
