# uart_protocol

## Purpose

This crate defines an enum which is the message sent between the remote card reader and the main unit.

These are as follows:

* SingleUid([u8;4]),

Read an NFC card with a 4 byte UID

* DoubleUid([u8;7]),


Read an NFC card with a 7 byte UID

* TripleUid([u8;10]),


Read an NFC card with a 10 byte UID

* ReadError,     

Failed to read a card

* ReaderFault,

Reader fault (eg not present)

* JustReset,

Sends on powerup (or if the rp2040 is reset by the watchdog - usually because the reader isn't present/faulty!)

* KeepAlive,

Routine keepalive message - nothing to see here