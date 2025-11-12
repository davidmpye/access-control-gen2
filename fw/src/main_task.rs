use embassy_time::{Timer};
use embassy_rp::gpio::Output;

use defmt::*;

use crate::remote_cardreader::CARDREADER_EVENT_SIGNAL;
use crate::remote_cardreader::CardReaderEvent;

use crate::database_task::{DATABASE_COMMAND_SIGNAL,DATABASE_RESPONSE_SIGNAL};
use crate::database_task::{DatabaseTaskCommand, DatabaseTaskResponse};


use crate::{CONFIG,config::LatchMode};

use crate::{LOG_EVENT_SIGNAL, LogEvent};

enum LatchState {
    Enabled,
    Disabled,
}

#[embassy_executor::task]
pub async fn main_task( mut relay_pin: Output<'static>, mut allowed_led: Output<'static>, mut denied_led: Output<'static>) -> ! {
    //The task needs ownership of Red LED, green LED, MOSFET_PIN

    //Receives message of new RFID read via signal, passes to database task.
    //Receives message from database task - card allowed, card denied
    //Activates appropriate LED +- FET
    //Sends message to telemetry task- to do
    let mut latch_state = LatchState::Disabled;

    loop {                
        //Await a message from the card reader handler
        match CARDREADER_EVENT_SIGNAL.wait().await {
            CardReaderEvent::CardMD5(hash) => {
                info!("Card hash received by main task");
                //Check if card valid
                DATABASE_COMMAND_SIGNAL.signal(DatabaseTaskCommand::CheckMD5Hash(hash));
                info!("Awaiting database task reply");
                let card_valid = match DATABASE_RESPONSE_SIGNAL.wait().await {
                    DatabaseTaskResponse::Found => {
                        true
                    },
                    _ => {
                        false
                    }
                };
                match CONFIG.latch_mode {
                    LatchMode::Latching => {
                        match latch_state {
                            LatchState::Disabled => {
                                if card_valid {
                                    info!("Card valid, access granted");
                                    relay_pin.set_high();
                                    allowed_led.set_high();
                                    latch_state = LatchState::Enabled;
                                    LOG_EVENT_SIGNAL.signal(LogEvent::ACTIVATED(hash));
                                }
                                else {
                                    info!("Card invalid, access denied");
                                    denied_led.set_high();
                                    Timer::after_secs(2).await;
                                    denied_led.set_low();
                                    LOG_EVENT_SIGNAL.signal(LogEvent::LOGINFAIL(hash));

                                }
                                //Todo - if we are using a remote cardreader, could we send a message back to it allowing it to illuminate a status LED too?
                            },
                            LatchState::Enabled => {
                                //Doesn't matter if card is valid, this counts as a sign out
                                info!("Signed out, device deactivated");
                                relay_pin.set_low();
                                allowed_led.set_low();
                                latch_state = LatchState::Disabled;
                                LOG_EVENT_SIGNAL.signal(LogEvent::DEACTIVATED(hash));
                            },
                        }
                    },
                    LatchMode::Timed(time) => {
                        if card_valid {
                            info!("Card valid, activating for {} seconds", time.as_secs());
                            relay_pin.set_high();
                            allowed_led.set_high();
                            Timer::after(time).await;
                            relay_pin.set_low();
                            allowed_led.set_low();
                            info!("Deactivated");
                            LOG_EVENT_SIGNAL.signal(LogEvent::ACTIVATED(hash));
                        }
                        else {
                            info!("Card invalid, access denied");
                            denied_led.set_high();
                            Timer::after_secs(2).await;
                            denied_led.set_low();
                            LOG_EVENT_SIGNAL.signal(LogEvent::LOGINFAIL(hash));

                        }
                    },
                }
            }
        }
        //Discard any pending message to avoid double-activation
        CARDREADER_EVENT_SIGNAL.reset();
    }
}

