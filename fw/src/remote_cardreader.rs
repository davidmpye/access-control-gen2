use embassy_rp::uart::{Uart, Config as UartConfig, InterruptHandler as UartInterruptHandler, Async};
use embassy_rp::peripherals::UART0;

use postcard::from_bytes_cobs;
use serde::{Serialize, Deserialize};

use defmt::*;

//The card reader messages we send
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
enum Message {
    CardSingleUid([u8;4]),
    CardDoubleUid([u8;7]),
    CardTripleUid([u8;10]),
    CardReadError,
    CardReaderFault,
}

#[embassy_executor::task]
pub async fn remote_cardreader_task(mut uart: Uart<'static, UART0, Async>) {
    loop {
        match read_card_cash(&mut uart).await {
            Ok(hash) => {
                let mut buf = [0x00u8; 32];
                match format_no_std::show(&mut buf, format_args!("{:032x}", hash)) {
                    Ok(str) => {
                        info!("Read card with MD5 hash {}", str);
                        //Send message to main task to confirm card read
                    }
                    Err(_e) => {
                        error!("Unable to format MD5 hash");
                    }
                }
            },
            Err(_) => {
                error!("Failed to read card hash");
            },
        }
    }
}

async fn read_card_cash<'d>(uart: &mut Uart<'d, UART0, Async>) -> Result<md5::Digest, ()> {
    let mut buf = [0x00u8;17];
    let mut count = 0x00usize;

    loop {
        //Keep reading bytes until Uart error, sentinel val (0x00) or buffer length reached
        if let Ok(_) = uart.read(&mut buf[count..count+1]).await {
            if buf[count] == 0x00 {
                //Message complete, cobs ensures 0x00 will never be part of message, just end marker
                break;
            }
            else {
                if count == buf.len() {
                    error!("Rx buffer overrun");
                    return Err(());
                }
                else {
                    count +=1;
                }
            }
        }
        else {
            error!("Uart Rx error");
            return Err(());
        }
    }

    let res: Result<Message, postcard::Error> = from_bytes_cobs(&mut buf[0..count]);
    if let Ok(message) = res {
        //Calculate the hash from the UID (4/7/10 bytes depending on card type)
        let hash = match message { 
            Message::CardSingleUid(data) => {
                debug!("Single UID - {}", data);
                md5::compute(data)
            },
            Message::CardDoubleUid(data) => {
                debug!("Double UID - {}", data);
                md5::compute(data)
            },
            Message::CardTripleUid(data)=> {
                debug!("Triple UID - {}", data);
                md5::compute(data)
            },
            _ => {
                error!("Received unhandled message from remote unit");
                return Err(());
            },
        };
        debug!("Computed hash : {}", hash);
        return Ok(hash)
    }
    else {
        error!("Message decode error from_bytes_cobs - bytes {}", buf[0..count]);
        return Err(());
    };
}

