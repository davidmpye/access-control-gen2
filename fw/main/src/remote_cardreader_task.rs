use embassy_rp::peripherals::UART0;
use embassy_rp::uart::{
    Async, Uart,
};

use defmt::*;

use postcard::from_bytes_cobs;

use serde::{Deserialize, Serialize};

use uart_protocol::RemoteMessage;

use crate::main_task::{CARDREADER_EVENT_SIGNAL, CardReaderEvent};

#[derive(Debug, Format)]
pub enum RemoteError {
    RxBufferOverflow, //Message exceeded rx buffer length
    UartError,        //Uart read error
    PostcardError,    //Unable to decode message using postcard - byte corruption?
}

#[embassy_executor::task]
pub async fn remote_cardreader_task(mut uart: Uart<'static, UART0, Async>) {
    loop {
        match read_message(&mut uart).await {
            Ok(msg) => match msg {
                RemoteMessage::SingleUid(data) => {
                    debug!("Single UID card - {}", data);
                    CARDREADER_EVENT_SIGNAL.signal(CardReaderEvent::CardMD5(md5::compute(data)));
                }
                RemoteMessage::DoubleUid(data) => {
                    debug!("Double UID card - {}", data);
                    CARDREADER_EVENT_SIGNAL.signal(CardReaderEvent::CardMD5(md5::compute(data)));
                }
                RemoteMessage::TripleUid(data) => {
                    debug!("Triple UID card - {}", data);
                    CARDREADER_EVENT_SIGNAL.signal(CardReaderEvent::CardMD5(md5::compute(data)));
                }
                RemoteMessage::ReadError => {
                    error!("Card read error");
                }
                RemoteMessage::ReaderFault => {
                    error!("Reader fault");
                }
                RemoteMessage::JustReset => {
                    //NB Happens at:
                    //initial power-on
                    //watchdog reset (eg if card reader hangs)
                    warn!("Reader just reset");
                }
                RemoteMessage::KeepAlive => {
                    debug!("Reader keepalive received");
                }
            },
            Err(e) => {
                error!("Remote error received - {}", e);
            }
        }
    }
}

async fn read_message<'d>(uart: &mut Uart<'d, UART0, Async>) -> Result<RemoteMessage, RemoteError> {
    let mut buf = [0x00u8; 16];

    for index in 0..buf.len() {
        if let Ok(_) = uart.read(&mut buf[index..index + 1]).await {
            if buf[index] == 0x00u8 {
                //Message complete, cobs ensures 0x00 will never be part of message, just end marker
                //Decode message using from_bytes_cobs from Postcard
                let res: Result<RemoteMessage, postcard::Error> = from_bytes_cobs(&mut buf[0..index]);
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
