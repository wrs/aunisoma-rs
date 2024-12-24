use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};

use crate::board::StatusLEDs;

pub struct Blinker {}

pub async fn blink(led: usize, delay_ms: u64) {
    loop {
        StatusLEDs::set(led);
        Timer::after(Duration::from_millis(delay_ms)).await;
        StatusLEDs::reset(led);
        Timer::after(Duration::from_millis(delay_ms)).await;
    }
}

#[embassy_executor::task]
pub async fn task() {
    blink(0, 100).await;
}

#[embassy_executor::task]
pub async fn task2() {
    blink(1, 105).await;
}

impl Blinker {
    pub fn spawn(self: Blinker, spawner: Spawner) {
        spawner.spawn(task()).unwrap();
        spawner.spawn(task2()).unwrap();
    }
}
