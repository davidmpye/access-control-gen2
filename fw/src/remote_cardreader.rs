use embassy_rp::uart::{Uart, Config as UartConfig, InterruptHandler as UartInterruptHandler, Async};
use embassy_rp::peripherals::UART0;

use postcard::from_bytes_cobs;
use serde::{Serialize, Deserialize};

use defmt::*;

//The card reader messages we send
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
enum Message {
    //RFID cards have different length UIDs
    SingleUid([u8;4]),
    DoubleUid([u8;7]),
    TripleUid([u8;10]),
    ReadError,
    ReaderFault,
    JustReset,
    KeepAlive,
}

#[derive(Debug, Format)]
pub enum RemoteError {
    BufferOverrun,  //Message exceeded rx buffer length
    UartError,      //Uart read error
    PostcardError,  //Unable to decode message using postcard - byte corruption?
}

#[embassy_executor::task]
pub async fn remote_cardreader_task(mut uart: Uart<'static, UART0, Async>) {
    loop {
        match read_message(&mut uart).await {
            Ok(msg) => match msg {
                Message::SingleUid(data) => {
                    debug!("Single UID card - {}", data);
                    handle_card_digest(md5::compute(data)).await;
                },
                Message::DoubleUid(data) => {
                    debug!("Double UID card - {}", data);
                    handle_card_digest(md5::compute(data)).await;
                },
                Message::TripleUid(data)=> {
                    debug!("Triple UID card - {}", data);
                    handle_card_digest(md5::compute(data)).await;
                },
                Message::ReadError => {
                    error!("Card read error");
                },
                Message::ReaderFault => {
                    error!("Reader fault");
                },
                Message::JustReset => {
                    //NB this may happen if the card reader hangs, and the watchdog resets the MCU. as well as at power on
                    info!("Reader just reset");
                }
                Message::KeepAlive => {
                    debug!("Reader keepalive received");
                },                
            }
            Err(e) => {
                error!("Remote error received - {}", e);
            }
        }
    }
}

async fn read_message<'d>(uart: &mut Uart<'d, UART0, Async>) -> Result<Message, RemoteError> {
    let mut buf = [0x00u8;16];

    for index in 0..buf.len() {
        if let Ok(_) = uart.read(&mut buf[index..index+1]).await {
            if buf[index] == 0x00u8 {
                //Message complete, cobs ensures 0x00 will never be part of message, just end marker
                //Decode message using from_bytes_cobs from Postcard
                let res: Result<Message, postcard::Error> = from_bytes_cobs(&mut buf[0..index]);
                match res {
                    Ok(message) => return Ok(message),
                    Err(_e) => return Err(RemoteError::PostcardError)
                }
            }
        }
        else {
            error!("Uart Rx error");
            return Err(RemoteError::UartError);
        }
    }
    //End of buffer hit.
    error!("Rx buffer overrun");
    return Err(RemoteError::BufferOverrun);
}

async fn handle_card_digest(digest: md5::Digest) {
    let mut buf = [0x00u8; 32];
    match format_no_std::show(&mut buf, format_args!("{:032x}", digest)) {
        Ok(str) => {
            info!("Read card with MD5 hash {}", str);
            //Send message to main task to confirm card read to 'do' something!
        },
        Err(_e) => {
            error!("Unable to format MD5 hash");
        }
    }
}
