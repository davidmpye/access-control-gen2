# uart_protocol

## Purpose

This crate defines the messaging protocol used between the main access control unit and a remote card reader unit (UART over RS485).
RS485 is used to try to avoid problems previously encountered with the MFRC522 misbehaving and refusing to read cards (likely due to misusing SPI bus over a number of metres!)

### RemoteMessage (from remote to main unit):

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

### MainMessage (from main unit to remote)

* AccessGranted,  

Put green LED on (if fitted) to indicate successful device/door activation

* AccessDenied,  

Put red LED on (if fitted) to indicate unsuccessful device/door activation

* AwaitingCard,  

Quiescent state (usually, both LEDs off)

