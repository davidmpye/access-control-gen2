#![no_std]

use serde::{Deserialize, Serialize};

//Messages from remote -> main unit
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum RemoteMessage {
    //RFID cards have different length UIDs
    SingleUid([u8; 4]),
    DoubleUid([u8; 7]),
    TripleUid([u8; 10]),
    ReadError,
    ReaderFault,
    JustReset,
    KeepAlive,
}

//Messages from main -> remote unit
//Main purpose of these is to allow the remote unit to show a status LED to the outside user
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum MainMessage {
    AccessGranted, //Put green LED on
    AccessDenied,  //Put red LED on
    AwaitingCard,  //No LED on, awating read
}
