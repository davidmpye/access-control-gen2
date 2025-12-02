use defmt::*;

use embassy_futures::select::{Either,select};
use embassy_rp::peripherals::UART0;
use embassy_rp::uart::{
    Async, Uart, UartRx, UartTx
};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::signal::Signal;

use heapless::Vec;
use postcard::{to_vec_cobs, from_bytes_cobs};
use serde::{Deserialize, Serialize};

use uart_protocol::{MainMessage,RemoteMessage};
use crate::main_task::{CARDREADER_EVENT_SIGNAL, CardReaderEvent};


#[derive(Debug, Format)]
pub enum RemoteError {
    RxBufferOverflow, //Message exceeded rx buffer length
    UartError,        //Uart read error
    PostcardError,    //Unable to decode message using postcard - byte corruption?
}

//Signal here to send a MainMessage to the remote unit
pub static MAIN_MESSAGE_SIGNAL: Signal<ThreadModeRawMutex, MainMessage> = Signal::new();

#[embassy_executor::task]
pub async fn remote_cardreader_task(mut uart: Uart<'static, UART0, Async>) {
    let (mut uart_tx, mut uart_rx) = uart.split();

    loop {
        match select(read_message(&mut uart_rx), MAIN_MESSAGE_SIGNAL.wait()).await {
            Either::First(msg) => {
                match msg {
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
            },
            Either::Second(msg) => {
                //Serialise and send this to the remote device
                debug!("Sending message to remote device");
                let vec: Vec<u8,16> = to_vec_cobs(&msg).unwrap();
                let _ = uart_tx.write(&vec).await;
            },
        }      
    }
}

async fn read_message<'d>(uart: &mut UartRx<'d, UART0, Async>) -> Result<RemoteMessage, RemoteError> {
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
