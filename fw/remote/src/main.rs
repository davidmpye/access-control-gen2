#![no_std]
#![no_main]

const DELAY_BETWEEN_READS:Duration = Duration::from_millis(2000);

use defmt::*;
use {defmt_rtt as _, panic_probe as _};

use embassy_executor::Spawner;
use embassy_rp::{gpio, gpio::{Level, Input, Output}, spi::{Config as SpiConfig,Spi}};
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;

use embassy_rp::peripherals::{UART0, WATCHDOG};
use embassy_rp::uart::{Uart, UartRx, Async, Config as UartConfig, InterruptHandler};
use embassy_rp::watchdog::Watchdog;

use embassy_rp::bind_interrupts;

use heapless::Vec;
use mfrc522::{comm::blocking::spi::SpiInterface, Mfrc522, Uid};
use postcard::{to_vec_cobs, from_bytes_cobs};

use uart_protocol::{RemoteMessage, MainMessage, MainMessage::*};

#[derive(Debug, Format)]
pub enum RemoteError {
    RxBufferOverflow, //Message exceeded rx buffer length
    UartError,        //Uart read error
    PostcardError,    //Unable to decode message using postcard - byte corruption?
}

bind_interrupts!(struct Irqs {
    UART0_IRQ => InterruptHandler<UART0>;
});

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

#[embassy_executor::task]
async fn led_task(mut uart_rx: UartRx<'static, UART0, Async>, mut red_led: Output<'static>, mut green_led: Output<'static>) -> ! {
    //This function 'owns' the two IOs as red/green LEDs.
    loop{
        match read_message(&mut uart_rx).await {
            Ok(message) => {
                match message {
                    AccessGranted => {
                        green_led.set_low();
                    },
                    AccessDenied => {
                        red_led.set_low();
                    },
                    AwaitingCard => {
                        green_led.set_high();
                        red_led.set_high();
                    },
                }
            },
            Err(e) => {
                error!("LED task encountered UART read error: {}", e);
            }
        }
    }
}


async fn read_message<'d>(uart: &mut UartRx<'d, UART0, Async>) -> Result<MainMessage,RemoteError> {
    let mut buf = [0x00u8; 16];

    for index in 0..buf.len() {
        if uart.read(&mut buf[index..index + 1]).await.is_ok() {
            if buf[index] == 0x00u8 {
                //Message complete, cobs ensures 0x00 will never be part of message, just end marker
                //Decode message using from_bytes_cobs from Postcard
                let res: Result<MainMessage, postcard::Error> = from_bytes_cobs(&mut buf[0..index]);
                match res {
                    Ok(message) => return Ok(message),
                    Err(_e) => return Err(RemoteError::PostcardError),
                }
            }
        } else {
            //Unclear of the circumstances when uart.read returns an Error
            error!("Uart Rx error");
            return Err(RemoteError::UartError);
        }
    }
    //If we are here, we have hit the end of the buffer
    error!("Rx buffer overflow");
    Err(RemoteError::RxBufferOverflow)
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
    
    //Set up the UART we use to speak over RS485 to the controller
    let (tx_pin, rx_pin, uart) = (p.PIN_0, p.PIN_1, p.UART0);
    let uart = Uart::new(uart, tx_pin, rx_pin, Irqs, p.DMA_CH0, p.DMA_CH1, UartConfig::default());

    let (mut uart_tx, uart_rx) = uart.split();

    //Leds on, 1 sec flash at power up
    let mut green_led = Output::new(p.PIN_3, Level::Low);
    let mut red_led = Output::new(p.PIN_2, Level::Low);
    Timer::after_millis(500).await;
    green_led.set_high();
    Timer::after_millis(500).await;
    red_led.set_high();
    //Spawn the status LED task, which owns the two GPIO ACC pins and the Rx half of the UART
    spawner.must_spawn(led_task(uart_rx, red_led, green_led));
    
    //This could be better - the newer embassy-rp watchdog is able to tell us if the reset is watchdog-origi
    debug!("Sending JustReset to controller");
    let vec: Vec<u8,16> = to_vec_cobs(&RemoteMessage::JustReset).unwrap();
    let _ = uart_tx.write(&vec).await;

    //Init SPI0 for talking to the card reader
    debug!("Init SPI0 peripheral");
    let (sck, mosi, miso, cs) = ( p.PIN_18, p.PIN_19, p.PIN_16, p.PIN_17);
    let cs = Output::new(cs, Level::High); //CS into output

    let spi0 = Spi::new_blocking(p.SPI0, sck, mosi, miso, SpiConfig::default());
    let mut spi0 = ExclusiveDevice::new(spi0, cs, Delay);

    //Nice idea to use MFRC IRQ pin but not supported by driver library presently
    let _irq = Input::new(p.PIN_20, gpio::Pull::Up);
    let mut rst = Output::new(p.PIN_21, Level::High);

    let mut last_sent_ok_message_counter = 0x00u8;

    debug!("Entering main loop");
    loop {   
        let interface = SpiInterface::new(&mut spi0);      
        //Reset, then try to initialise the MFRC522 readerS
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
                                    let mut buf = [0x00;4];
                                    buf.copy_from_slice(&inner.as_bytes()[..4]);
                                    RemoteMessage::SingleUid(buf)
                                },
                                Ok(ref _uid @ Uid::Double(ref inner)) => {
                                    debug!("Double UID card read");
                                    let mut buf = [0x00;7];
                                    buf.copy_from_slice(&inner.as_bytes()[..7]);
                                    RemoteMessage::DoubleUid(buf)
                                },
                                Ok(ref _uid @ Uid::Triple(ref inner)) => {
                                    debug!("Triple UID card read");

                                    let mut buf = [0x00;10];
                                    buf.copy_from_slice(&inner.as_bytes()[..10]);
                                    RemoteMessage::TripleUid(buf)
                                },
                                Err(_e) => {
                                    error!("MFRC select error");
                                    RemoteMessage::ReadError
                                },
                            };
                            let vec: Vec<u8,16> = to_vec_cobs(&message).unwrap();
                            let _ = uart_tx.write(&vec).await;
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
                                //Send OK message to main unit so it knows we're still alive
                                let vec: Vec<u8,16> = to_vec_cobs(&RemoteMessage::KeepAlive).unwrap();
                                let _ = uart_tx.write(&vec).await;
                                last_sent_ok_message_counter = 0;
                            }
                        }
                    } 
                },
                Err(_e) => {
                    error!("Device init failed, waiting to retry");
                    let vec: Vec<u8,16> = to_vec_cobs(&RemoteMessage::ReaderFault).unwrap();
                    let _ = uart_tx.write(&vec).await;
                    Timer::after_millis(500).await;
                }
        }
    }
}        
