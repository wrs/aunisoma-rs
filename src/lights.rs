use embassy_stm32::time::Hertz;
use embassy_stm32::timer::low_level::CountingMode;
use embassy_stm32::timer::simple_pwm::{Ch1, Ch2, Ch4, PwmPin, SimplePwm};
use embassy_time::Timer;

use crate::board::LedTimer;

#[embassy_executor::task]
pub(crate) async fn lights_task(
    timer: LedTimer,
    led_red: PwmPin<'static, LedTimer, Ch1>,
    led_green: PwmPin<'static, LedTimer, Ch2>,
    led_blue: PwmPin<'static, LedTimer, Ch4>,
) {
    defmt::info!("led_pwm_task started");

    let pwm = SimplePwm::new(
        timer,
        Some(led_red),
        Some(led_green),
        None,
        Some(led_blue),
        Hertz(1000),
        CountingMode::EdgeAlignedUp,
    );
    let mut channels = pwm.split();
    channels.ch1.enable();
    channels.ch2.enable();
    channels.ch4.enable();

    let mut brightness: u8 = 0;
    loop {
        channels.ch1.set_duty_cycle_fraction(brightness as u16, 255);
        channels.ch2.set_duty_cycle_fraction(brightness as u16, 255);
        channels.ch4.set_duty_cycle_fraction(brightness as u16, 255);
        brightness = brightness.wrapping_add(1);
        Timer::after_millis(10).await;
    }
}
