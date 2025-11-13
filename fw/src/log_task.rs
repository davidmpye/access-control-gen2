use embassy_net::{
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
    Stack,
};

use embassy_rp::clocks::RoscRng;

use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;

use defmt::*;

use embassy_time::{Timer, WithTimeout};
use log::Log;
use rand::RngCore;

use reqwless::client::{HttpClient, TlsConfig, TlsVerify};
use reqwless::request::Method;
use reqwless::{request::RequestBuilder, response::StatusCode};

use crate::CONFIG;

pub(crate) enum LogEvent {
    ACTIVATED([u8; 32]),
    DEACTIVATED([u8; 32]),
    LOGINFAIL([u8; 32]),
    ERROR,
}

//The queue can hold 32 events awaiting logging
pub static LOG_EVENT_QUEUE: Channel<ThreadModeRawMutex, LogEvent, 32> =
    Channel::<ThreadModeRawMutex, LogEvent, 32>::new();

pub struct LogTaskRunner {
    stack: Stack<'static>,
}

impl LogTaskRunner {
    pub fn new(stack: Stack<'static>) -> Self {
        Self { stack }
    }

    pub async fn run(self) -> ! {
        loop {
            //If we have no wifi connection, retry in 60 seconds
            while !self.stack.is_config_up() {
                warn!("Log task waiting - no wifi connection");
                //Wait 60 seconds and try again.
                Timer::after_secs(60).await;
            }

            //Await an event from the queue
            let event = LOG_EVENT_QUEUE.receive().await;
            match self.log_event(&event)
                .with_timeout(CONFIG.http_timeout)
                .await {
                    Ok(result) => {
                        match result {
                            Ok(_) => {
                                info!("Log event recorded successfully")
                            }
                            Err(_) => {
                                warn!("Log event failed, will be requeued");
                                //Attempt to requeue
                                if let Err(_e) = LOG_EVENT_QUEUE.try_send(event) {
                                    error!("Unable to requeue - this event will be lost")
                                }
                            }
                        }
                    },
                    Err(_timeout)=> {
                        warn!("Log event timeout, will be requeued");
                        //Attempt to requeue
                        if let Err(_e) = LOG_EVENT_QUEUE.try_send(event) {
                            error!("Unable to requeue - this event will be lost")
                        }
                    },
                }
        }
    }

    async fn log_event(&self, event: &LogEvent) -> Result<(), ()> {
        //Convert hash to ascii string representation
        let hash = match event {
            LogEvent::ACTIVATED(hash) | LogEvent::DEACTIVATED(hash) | LogEvent::LOGINFAIL(hash) => {
                //Convert hash to an ascii str representation
                hash
            }
            _ => b"N/A                             ",  //32 bytes long too...
        };
        let hash = core::str::from_utf8(hash).unwrap_or(" Non-ascii bytes in hash");

        //Get printable name for event, as expected by the Makerspace logging API
        let event_str = match event {
            LogEvent::ACTIVATED(_) => "Activated",
            LogEvent::DEACTIVATED(_) => "Deactivated",
            LogEvent::LOGINFAIL(_) => "LoginFail",
            LogEvent::ERROR => "ERROR",
        };

        //Build a fresh http client for each database update attempt
        info!("Log task making connection to log endpoint");
        let mut tls_read_buffer = [0; 8096];
        let mut tls_write_buffer = [0; 8096];
        let mut rng = RoscRng;
        let seed = rng.next_u64();

        let client_state = TcpClientState::<2, 1024, 1024>::new();
        let tcp_client = TcpClient::new(self.stack, &client_state);
        let dns_client = DnsSocket::new(self.stack);
        let tls_config = TlsConfig::new(
            seed,
            &mut tls_read_buffer,
            &mut tls_write_buffer,
            TlsVerify::None,
        );
        let mut http_client = HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);

        let mut url_buf = [0x00u8; 128];
        let url = format_no_std::show(
            &mut url_buf,
            format_args!(
                "{}/{}/{}",
                CONFIG.url_endpoint, CONFIG.device_name, CONFIG.log_prefix
            ),
        )
        .expect("Unable to build DB update URL");
        debug!("Connecting to {}", &url);

        let mut json_buf = [0x00; 256];
        let json = format_no_std::show(
            &mut json_buf,
            format_args!("{{ \"type\": \"{}\", \"hash\": \"{}\"}}", event_str, hash),
        )
        .expect("Unable to build JSON log event");

        debug!("Json string: {}", json);

        let mut rx_buf = [0x00; 512];

        let x = if let Ok(req) = http_client.request(Method::POST, &url).await {
            if let Ok(_e) = req
                .content_type(reqwless::headers::ContentType::ApplicationJson)
                .body(json.as_bytes())
                .send(&mut rx_buf)
                .await
            {
                Ok(())
            } else {
                error!("HTTP response error");
                Err(())
            }
        } else {
            error!("Client request error");
            Err(())
        };
        x
    }

}
