use embassy_rp::gpio::Output;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;

use defmt::*;

use crate::database_task::{DatabaseTaskCommand, DatabaseTaskResponse};
use crate::database_task::{DATABASE_COMMAND_SIGNAL, DATABASE_RESPONSE_SIGNAL};

use crate::remote_cardreader_task::MAIN_MESSAGE_SIGNAL;
use crate::{config::LatchMode, CONFIG};

use crate::{LogEvent, LOG_EVENT_QUEUE};

pub (crate) enum CardReaderEvent {
    CardMD5(md5::Digest),
}

pub (crate) static CARDREADER_EVENT_SIGNAL: Signal<ThreadModeRawMutex, CardReaderEvent> = Signal::new();

enum LatchState {
    Enabled([u8;32]), //We store the card hash of the person who is signed into the controller
    Disabled,
}

#[embassy_executor::task]
pub async fn main_task(
    mut relay_pin: Output<'static>,
    mut allowed_led: Output<'static>,
    mut denied_led: Output<'static>,
) -> ! {
    //The task needs ownership of Red LED, Green LED, MOSFET gate pin
    
    //Receives message of new RFID read via signal, passes to database task.
    //Receives message from database task - card allowed, card denied
    //Activates appropriate LED +- FET
    //Sends message to the log task queue so it can update the backend
    let mut latch_state = LatchState::Disabled;

    loop {
        //Await a message from the card reader handler
        match CARDREADER_EVENT_SIGNAL.wait().await {
            CardReaderEvent::CardMD5(digest) => {
                //Format the digest into an ascii string, as that's what we currently use as the in-flash representation (legacy!)             
                let mut hash_buf = [0x00u8; 32];
                let hash_str = match format_no_std::show(&mut hash_buf, format_args!("{:032x}", digest)) {
                    Ok(str) => {
                        str
                    }
                    Err(_e) => {
                        error!("Unable to format MD5 hash");
                        continue;
                    }
                };
                info!("Card read with hash {}", hash_str);
                //Check if card valid - we use the hash_buf to avoid lifetime issues
                DATABASE_COMMAND_SIGNAL.signal(DatabaseTaskCommand::CheckMD5Hash(hash_buf));
                info!("Awaiting database task reply");
                let card_valid = match DATABASE_RESPONSE_SIGNAL.wait().await {
                    DatabaseTaskResponse::Found => true,
                    _ => false,
                };
                
                match CONFIG.latch_mode {
                    LatchMode::Latching => {
                        match latch_state {
                            LatchState::Disabled => {
                                if card_valid {
                                    info!("Card valid, access granted");
                                    relay_pin.set_high();
                                    allowed_led.set_high();
                                    latch_state = LatchState::Enabled(hash_buf);
                                    queue_log_message(LogEvent::ACTIVATED(hash_buf));
                                    //If we have a remote cardreader, it will set LED to green
                                    MAIN_MESSAGE_SIGNAL.signal(uart_protocol::MainMessage::AccessGranted);
                                } else {
                                    info!("Card invalid, access denied");
                                    denied_led.set_high();
                                    //Remote cardreader LED to red
                                    MAIN_MESSAGE_SIGNAL.signal(uart_protocol::MainMessage::AccessDenied);
                                    Timer::after_secs(2).await;
                                    denied_led.set_low();
                                    queue_log_message(LogEvent::LOGINFAIL(hash_buf));
                                    //Turn off remote cardreader LED (if present)
                                    MAIN_MESSAGE_SIGNAL.signal(uart_protocol::MainMessage::AwaitingCard);
                                }
                            }
                            LatchState::Enabled(hash) => {
                                //Doesn't matter if card is valid, this counts as a sign out
                                info!("Signed out, device deactivated");
                                relay_pin.set_low();
                                allowed_led.set_low();
                                latch_state = LatchState::Disabled;
                                queue_log_message(LogEvent::DEACTIVATED(hash));
                            }
                        }
                    }
                    LatchMode::Timed(time) => {
                        if card_valid {
                            info!("Card valid, latching for {} seconds", time.as_secs());
                            relay_pin.set_high();
                            allowed_led.set_high();
                            MAIN_MESSAGE_SIGNAL.signal(uart_protocol::MainMessage::AccessGranted);
                            Timer::after(time).await;
                            relay_pin.set_low();
                            allowed_led.set_low();
                            MAIN_MESSAGE_SIGNAL.signal(uart_protocol::MainMessage::AwaitingCard);
                            debug!("Deactivated");
                            queue_log_message(LogEvent::ACTIVATED(hash_buf));
                        } else {
                            info!("Card invalid, access denied");
                            denied_led.set_high();
                            MAIN_MESSAGE_SIGNAL.signal(uart_protocol::MainMessage::AccessDenied);
                            Timer::after_secs(2).await;
                            denied_led.set_low();
                            MAIN_MESSAGE_SIGNAL.signal(uart_protocol::MainMessage::AwaitingCard);
                            queue_log_message(LogEvent::LOGINFAIL(hash_buf));
                        }
                    }
                }
            }
        }
        //Discard any pending message to avoid double-activation
        CARDREADER_EVENT_SIGNAL.reset();
    }
}

fn queue_log_message(e: LogEvent) {
    match LOG_EVENT_QUEUE.try_send(e) {
        Ok(_) => {
            debug!("Log event added to logger queue");
        }
        Err(_) => {
            error!("Log event queue full, event will be lost");
        }
    }
}
