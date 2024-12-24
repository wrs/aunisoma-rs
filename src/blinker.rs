use cortex_m::singleton;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output};
use embassy_time::{Duration, Timer};

pub struct Blinker {
    pub led: Output<'static>,
    pub led2: Output<'static>,
}

#[embassy_executor::task]
pub async fn task(led: &'static mut Output<'static>) {
    loop {
        led.set_level(Level::High);
        Timer::after(Duration::from_millis(100)).await;
        led.set_level(Level::Low);
        Timer::after(Duration::from_millis(100)).await;
    }
}

#[embassy_executor::task]
pub async fn task2(led: &'static mut Output<'static>) {
    loop {
        led.set_level(Level::High);
        Timer::after(Duration::from_millis(105)).await;
        led.set_level(Level::Low);
        Timer::after(Duration::from_millis(105)).await;
    }
}

impl Blinker {
    pub fn spawn(self: Blinker, spawner: Spawner) {
        let blinker = singleton!(BLINKER: Blinker = self).unwrap();
        spawner.spawn(task(&mut blinker.led)).unwrap();
        spawner.spawn(task2(&mut blinker.led2)).unwrap();
    }
}
