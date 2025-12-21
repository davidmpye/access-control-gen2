use embassy_time::{Delay, Duration, Timer};

use defmt::*;

use mfrc522::{comm::blocking::spi::SpiInterface, Mfrc522, Uid};

use crate::{
    main_task::{CardReaderEvent, CARDREADER_EVENT_SIGNAL},
    Spi0Resources,
};

use embassy_rp::spi::{Config as SpiConfig, Spi};

use embedded_hal_bus::spi::ExclusiveDevice;

use embassy_rp::gpio::{Level, Output};

#[embassy_executor::task]
pub async fn local_cardreader_task(spi: Spi0Resources) -> ! {
    let spi0 = Spi::new_blocking(spi.spi, spi.sck, spi.mosi, spi.miso, SpiConfig::default());
    let mut spi0: ExclusiveDevice<
        Spi<'_, embassy_rp::peripherals::SPI0, embassy_rp::spi::Blocking>,
        Output,
        Delay,
    > = ExclusiveDevice::new(spi0, Output::new(spi.cs, Level::High), Delay);
    let mut rst = Output::new(spi.rst, Level::High);

    loop {
        //Set up the MFRC522 SPI Interface
        debug!("Initialising MFRC522 SPI interface");
        let interface = SpiInterface::new(&mut spi0);
        //Reset, then try to initialise the MFRC522 readerS
        //Pull rst low for 250mS
        rst.set_low();
        Timer::after(Duration::from_millis(250)).await;
        rst.set_high();

        match Mfrc522::new(interface).init() {
            Ok(mut mfrc) => {
                info!("MFRC522 init OK");
                loop {
                    //If the MFRC disappears or goes into a fault state wupa() blocks,
                    //and we have to rely on the watchdog to restart the controller
                    if let Ok(atqa) = mfrc.wupa() {
                        debug!("AtqA select");
                        match mfrc.select(&atqa) {
                            Ok(ref _uid @ Uid::Single(ref inner)) => {
                                debug!("Single UID card read");
                                CARDREADER_EVENT_SIGNAL.signal(CardReaderEvent::CardMD5(
                                    md5::compute(&inner.as_bytes()[..4]),
                                ))
                            }
                            Ok(ref _uid @ Uid::Double(ref inner)) => {
                                debug!("Double UID card read");
                                CARDREADER_EVENT_SIGNAL.signal(CardReaderEvent::CardMD5(
                                    md5::compute(&inner.as_bytes()[..7]),
                                ))
                            }
                            Ok(ref _uid @ Uid::Triple(ref inner)) => {
                                debug!("Triple UID card read");
                                CARDREADER_EVENT_SIGNAL.signal(CardReaderEvent::CardMD5(
                                    md5::compute(&inner.as_bytes()[..10]),
                                ))
                            }
                            Err(_e) => {
                                error!("MFRC select error");
                            }
                        };
                    }
                    //Wait 100mS between read attempts
                    Timer::after_millis(100).await;
                }
            }
            Err(_) => {
                error!("MFRC init failed, will retry in 10s");
                Timer::after_secs(10).await;
            }
        }
    }
}
