use crate::StatusLEDs;
use embassy_futures::join::join;
use embassy_time::{Duration, Timer};

pub async fn blink(led: usize, delay_ms: u64) {
    loop {
        StatusLEDs::set(led);
        Timer::after(Duration::from_millis(delay_ms)).await;
        StatusLEDs::reset(led);
        Timer::after(Duration::from_millis(delay_ms)).await;
    }
}

pub async fn task() {
    defmt::info!("blink task started");
    blink(0, 100).await;
}

pub async fn task2() {
    defmt::info!("blink task2 started");
    blink(1, 105).await;
}

#[embassy_executor::task]
pub(crate) async fn blinker_task() {
    join(task(), task2()).await;
}
