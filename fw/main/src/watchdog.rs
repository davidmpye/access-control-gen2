use embassy_rp::gpio::{Output, Level};
use embassy_rp::watchdog::Watchdog;
use embassy_time::{Duration, Timer};

use crate::WatchdogResources;

use defmt::*;

const WATCHDOG_TIMER_MS: u64 = 2500;
const WATCHDOG_FEED_TIMER_MS: u64 = 200;
const LED_BLINK_TIME_MS: u64 = 2;

#[embassy_executor::task]
pub async fn watchdog_task(resources: WatchdogResources) -> ! {
    let mut dog = Watchdog::new(resources.dog);
    let mut heartbeat_led = Output::new( resources.heartbeat_led, Level::Low);

    dog.start(Duration::from_millis(WATCHDOG_TIMER_MS));
    info!("Watchdog enabled");
    loop {
        dog.feed();
        Timer::after(Duration::from_millis(WATCHDOG_FEED_TIMER_MS)).await;
        heartbeat_led.set_high();
        Timer::after(Duration::from_millis(LED_BLINK_TIME_MS)).await;
        heartbeat_led.set_low();
    }
}
