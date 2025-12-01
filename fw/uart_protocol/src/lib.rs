#![no_std]

use serde::{Serialize, Deserialize};

//The card reader messages we send to the main unit
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum RemoteMessage {
    //RFID cards have different length UIDs
    SingleUid([u8;4]),
    DoubleUid([u8;7]),
    TripleUid([u8;10]),
    ReadError,
    ReaderFault,
    JustReset,
    KeepAlive,
}


