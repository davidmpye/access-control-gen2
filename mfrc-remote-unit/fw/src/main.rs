#![no_std]
#![no_main]

const DELAY_BETWEEN_READS:Duration = Duration::from_millis(2000);

use defmt::*;
use {defmt_rtt as _, panic_probe as _};

use embassy_executor::Spawner;
use embassy_rp::{gpio, gpio::{Level, Input, Output}, spi::{self,Spi}};
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;

use embassy_rp::peripherals::{UART0, WATCHDOG};
use embassy_rp::uart::{Uart, Config as UartConfig, InterruptHandler};
use embassy_rp::watchdog::Watchdog;

use embassy_rp::bind_interrupts;

use heapless::Vec;
use mfrc522::{comm::blocking::spi::SpiInterface, Mfrc522, Uid};
use postcard::to_vec_cobs;
use serde::{Serialize, Deserialize};

bind_interrupts!(struct Irqs {
    UART0_IRQ => InterruptHandler<UART0>;
});

//The card reader messages we send to the main unit
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
enum Message {
    SingleUid([u8;4]),
    DoubleUid([u8;7]),
    TripleUid([u8;10]),
    ReadError,
    ReaderFault,
    ReaderOk
}

const WATCHDOG_TIMER_SECS:u64 = 2;
const WATCHDOG_FEED_TIMER_MS:u64 = 250;

#[embassy_executor::task]
pub async fn watchdog_task(watchdog: WATCHDOG, mut led: Output<'static>) -> ! {
    let mut dog = Watchdog::new(watchdog);
    dog.start(Duration::from_secs(WATCHDOG_TIMER_SECS));
    loop {        
        dog.feed();
        Timer::after(Duration::from_millis(WATCHDOG_FEED_TIMER_MS)).await;
        led.toggle();
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    // Initialise Peripherals
    let p = embassy_rp::init(Default::default());

    //Spawn the watchdog task first

    let heartbeat_led = Output::new(p.PIN_7, Level::Low);
    spawner.must_spawn(watchdog_task(p.WATCHDOG, heartbeat_led));

    //Set up pins
    let mut card_read_led = Output::new(p.PIN_6, Level::Low);

    let miso = p.PIN_16;
    let ss =p.PIN_17;
    let sck = p.PIN_18;
    let mosi = p.PIN_19;
    //Nice idea to use MFRC IRQ but not supported by driver
    let _irq = Input::new(p.PIN_20, gpio::Pull::Up);
    let mut rst = Output::new(p.PIN_21, Level::High);
    let cs = Output::new(ss, Level::High);

    let (tx_pin, rx_pin, uart) = (p.PIN_0, p.PIN_1, p.UART0);

    let spi = Spi::new_blocking(p.SPI0, sck, mosi, miso, spi::Config::default());
    let mut spi = ExclusiveDevice::new(spi, cs, Delay);

    let mut uart = Uart::new(uart, tx_pin, rx_pin, Irqs, p.DMA_CH0, p.DMA_CH1, UartConfig::default());

    let mut last_sent_ok_message_counter = 0x00u8;

    loop {   
        let interface = SpiInterface::new(&mut spi);      
        //Reset, then initialise the MFRC522
        //Pull rst low for 250mS
        rst.set_low();
        Timer::after(Duration::from_millis(250)).await;
        rst.set_high();
        match Mfrc522::new(interface).init() {
                Ok(mut mfrc) => {
                    //Try to read a card for 10 seconds
                    loop {
                        //If the MFRC disappears or goes into a fault state wupa() blocks, 
                        //and we have to rely on the watchdog to restart us
                        if let Ok(atqa) = mfrc.wupa() {
                            debug!("AtqA select");
                            let message = match mfrc.select(&atqa) {
                                Ok(ref _uid @ Uid::Single(ref inner)) => {
                                    debug!("Single UID card read");
                                    let bytes = inner.as_bytes();
                                    Message::SingleUid([bytes[0], bytes[1], bytes[2], bytes[3]])
                                },
                                Ok(ref _uid @ Uid::Double(ref inner)) => {
                                    debug!("Double UID card read");
                                    let bytes = inner.as_bytes();
                                    Message::DoubleUid([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6]])
                                },
                                Ok(ref _uid @ Uid::Triple(ref inner)) => {
                                    debug!("Triple UID card read");
                                    let bytes = inner.as_bytes();
                                    Message::TripleUid([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9]])
                                },
                                Err(_e) => {
                                    error!("MFRC select error");
                                    Message::ReadError
                                },
                            };
                            let vec: Vec<u8,16> = to_vec_cobs(&message).unwrap();
                            let _ = uart.write(&vec).await;
                            debug!("Card UID message sent");
                            //Flash the "card read" LED to indicate success here.
                            card_read_led.set_high();
                            Timer::after_millis(100).await;                                    
                            card_read_led.set_low();
                            Timer::after_millis(100).await;
                            Timer::after(DELAY_BETWEEN_READS).await;  
                        }
                        else {
                            //WUPA failed, no card found. Wait 100mS in between read attempts
                            Timer::after_millis(100).await;
                            last_sent_ok_message_counter += 1;
                            if last_sent_ok_message_counter == 10 {
                                //Send OK message to main unit so it knows we're still alive and reset counter
                                let vec: Vec<u8,16> = to_vec_cobs(&Message::ReaderOk).unwrap();
                                let _ = uart.write(&vec).await;
                                last_sent_ok_message_counter = 0;
                            }
                        }
                    } 
                },
                Err(_e) => {
                    error!("Device init failed, waiting to retry");
                    let vec: Vec<u8,16> = to_vec_cobs(&Message::ReaderFault).unwrap();
                    let _ = uart.write(&vec).await;
                    Timer::after_millis(500).await;
                }
        }
    }
}        
