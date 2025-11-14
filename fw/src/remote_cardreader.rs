use embassy_rp::peripherals::UART0;
use embassy_rp::uart::{
    Async, Config as UartConfig, InterruptHandler as UartInterruptHandler, Uart,
};

use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::signal::Signal;

use postcard::from_bytes_cobs;
use serde::{Deserialize, Serialize};

use defmt::*;

//The card reader messages we send
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
enum Message {
    //RFID cards have different length UIDs
    SingleUid([u8; 4]),
    DoubleUid([u8; 7]),
    TripleUid([u8; 10]),
    ReadError,
    ReaderFault,
    JustReset,
    KeepAlive,
}

#[derive(Debug, Format)]
pub enum RemoteError {
    RxBufferOverflow, //Message exceeded rx buffer length
    UartError,        //Uart read error
    PostcardError,    //Unable to decode message using postcard - byte corruption?
}

pub enum CardReaderEvent {
    CardMD5(md5::Digest),
}

pub static CARDREADER_EVENT_SIGNAL: Signal<ThreadModeRawMutex, CardReaderEvent> = Signal::new();

#[embassy_executor::task]
pub async fn remote_cardreader_task(mut uart: Uart<'static, UART0, Async>) {
    loop {
        match read_message(&mut uart).await {
            Ok(msg) => match msg {
                Message::SingleUid(data) => {
                    debug!("Single UID card - {}", data);
                    CARDREADER_EVENT_SIGNAL.signal(CardReaderEvent::CardMD5(md5::compute(data)));
                }
                Message::DoubleUid(data) => {
                    debug!("Double UID card - {}", data);
                    CARDREADER_EVENT_SIGNAL.signal(CardReaderEvent::CardMD5(md5::compute(data)));
                }
                Message::TripleUid(data) => {
                    debug!("Triple UID card - {}", data);
                    CARDREADER_EVENT_SIGNAL.signal(CardReaderEvent::CardMD5(md5::compute(data)));
                }
                Message::ReadError => {
                    error!("Card read error");
                }
                Message::ReaderFault => {
                    error!("Reader fault");
                }
                Message::JustReset => {
                    //NB this may happen if the card reader hangs,
                    //and the watchdog resets the MCU, as well as at initial power on
                    warn!("Reader just reset");
                }
                Message::KeepAlive => {
                    debug!("Reader keepalive received");
                }
            },
            Err(e) => {
                error!("Remote error received - {}", e);
            }
        }
    }
}

async fn read_message<'d>(uart: &mut Uart<'d, UART0, Async>) -> Result<Message, RemoteError> {
    let mut buf = [0x00u8; 16];

    for index in 0..buf.len() {
        if let Ok(_) = uart.read(&mut buf[index..index + 1]).await {
            if buf[index] == 0x00u8 {
                //Message complete, cobs ensures 0x00 will never be part of message, just end marker
                //Decode message using from_bytes_cobs from Postcard
                let res: Result<Message, postcard::Error> = from_bytes_cobs(&mut buf[0..index]);
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
    return Err(RemoteError::RxBufferOverflow);
}
