use embassy_rp::watchdog::Watchdog;
use embassy_rp::peripherals::WATCHDOG;
use embassy_time::{Duration, Timer};
use embassy_rp::gpio::Output;

use defmt::*;

const WATCHDOG_TIMER_MS:u64 = 2500;
const WATCHDOG_FEED_TIMER_MS:u64 = 200;
const LED_BLINK_TIME_MS:u64 = 2;

#[embassy_executor::task]
pub async fn watchdog_task(watchdog: WATCHDOG, mut heartbeat_pin: Output<'static>) -> ! {
    let mut dog = Watchdog::new(watchdog);
    dog.start(Duration::from_millis(WATCHDOG_TIMER_MS));
    info!("Watchdog enabled");
    loop {        
        dog.feed();
        Timer::after(Duration::from_millis(WATCHDOG_FEED_TIMER_MS)).await;
        heartbeat_pin.set_high();
        Timer::after(Duration::from_millis(LED_BLINK_TIME_MS)).await;
        heartbeat_pin.set_low();
    }
}