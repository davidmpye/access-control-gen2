use embassy_rp::peripherals::SPI0;
use embassy_rp::spi::{Blocking, Config as SpiConfig, Spi};

use embassy_rp::gpio::AnyPin;
use embedded_hal::digital::OutputPin;
use embedded_hal::spi::SpiDevice;

use defmt::*;

use mfrc522::{comm::blocking::spi::SpiInterface, Mfrc522, Uid};

use crate::remote_cardreader_task::{CardReaderEvent, CARDREADER_EVENT_SIGNAL};

use embassy_time::{Duration, Timer};

pub struct LocalCardreaderTaskRunner<T: SpiDevice, U: OutputPin> {
    device: T,
    rst: U,
}

impl<T: SpiDevice, U: OutputPin> LocalCardreaderTaskRunner<T, U> {
    pub fn new(device: T, rst: U) -> Self {
        Self { device, rst }
    }

    pub async fn run(&mut self) -> ! {
        loop {
            //Set up the MFRC522 SPI Interface
            debug!("Initialising MFRC522 SPI interface");
            let interface: SpiInterface<&mut T, mfrc522::comm::blocking::spi::DummyDelay> =
                SpiInterface::new(&mut self.device);
            //Reset, then try to initialise the MFRC522 readerS
            //Pull rst low for 250mS
            let _ = self.rst.set_low();
            Timer::after(Duration::from_millis(250)).await;
            let _ = self.rst.set_high();

            if let Ok(mut mfrc) = Mfrc522::new(interface).init() {
                info!("MFRC522 init OK");

                loop {
                    //If the MFRC disappears or goes into a fault state wupa() blocks,
                    //and we have to rely on the watchdog to restart the controller
                    if let Ok(atqa) = mfrc.wupa() {
                        debug!("AtqA select");
                        match mfrc.select(&atqa) {
                            Ok(ref _uid @ Uid::Single(ref inner)) => {
                                debug!("Single UID card read");
                                CARDREADER_EVENT_SIGNAL
                                    .signal(CardReaderEvent::CardMD5(md5::compute(&inner.as_bytes()[..4])))
                            }
                            Ok(ref _uid @ Uid::Double(ref inner)) => {
                                debug!("Double UID card read");
                                CARDREADER_EVENT_SIGNAL
                                    .signal(CardReaderEvent::CardMD5(md5::compute(&inner.as_bytes()[..7])))
                            }
                            Ok(ref _uid @ Uid::Triple(ref inner)) => {
                                debug!("Triple UID card read");
                                CARDREADER_EVENT_SIGNAL
                                    .signal(CardReaderEvent::CardMD5(md5::compute(&inner.as_bytes()[..10])))
                            }
                            Err(_e) => {
                                error!("MFRC select error");
                            }
                        };
                    }
                    //Wait 100mS between read attempts
                    Timer::after_millis(100).await;
                }
            } else {
                error!("MFRC init failed, will retry in 10s");
                Timer::after_secs(10).await;
            }
        }
    }
}
