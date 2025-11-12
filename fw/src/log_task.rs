use embassy_net::{Stack,dns::DnsSocket, tcp::client::{TcpClient, TcpClientState} };

use embassy_rp::clocks::RoscRng;

use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::signal::Signal;

use defmt::*;

use rand::RngCore;

use reqwless::{request::RequestBuilder, response::StatusCode};
use reqwless::client::{HttpClient, TlsConfig, TlsVerify};
use reqwless::request::Method;

use crate::CONFIG;

pub(crate) enum LogEvent {
    ACTIVATED([u8;32]),
    DEACTIVATED([u8;32]),
    LOGINFAIL([u8;32]),
    ERROR,
}

pub static LOG_EVENT_SIGNAL: Signal<ThreadModeRawMutex, LogEvent> =
    Signal::new();


pub struct LogTaskRunner {
    stack: Stack<'static>,
}

impl LogTaskRunner {
    pub fn new(stack: Stack<'static>) -> Self {
        Self { stack }
    }

    pub async fn run(self) -> ! {
        loop {
            let event = LOG_EVENT_SIGNAL.wait().await;

            //Obtain the two strings we nede to send to the log
            let hash = match event {
                LogEvent::ACTIVATED(hash)  | LogEvent::DEACTIVATED(hash) | LogEvent::LOGINFAIL(hash) => {
                    hash
                }
                _ => {
                    *b"NO HASH                         "
                }
            };

            let event = match event {
                LogEvent::ACTIVATED(_) => {
                    "Activated"
                },
                LogEvent::DEACTIVATED(_) => {
                    "Deactivated"
                },
                LogEvent::LOGINFAIL(_) => {
                    "LoginFail"
                }
                LogEvent::ERROR => {
                    "ERROR"
                }
            };

            let hash_as_str = core::str::from_utf8(&hash).unwrap_or(" Invalid bytes");

            //Build an http client and send the log event
              //Check if network is up, abort if not
            if !self.stack.is_config_up() {
                error!("Unable to sync - no wifi connection");
            }

            //Build a fresh http client for each database update attempt
            info!("Reqwless HTTP client init");
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
            let mut http_client= HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);

            let mut url_buf = [0x00u8;128];
            let url = format_no_std::show(&mut url_buf, format_args!("{}/{}/{}", CONFIG.url_endpoint, CONFIG.device_name, CONFIG.log_prefix)).expect("Unable to build DB update URL");
            debug!("Connecting to {}", &url);

            let mut json_buf = [0x00;256];
            let json = format_no_std::show(&mut json_buf, format_args!("{{ \"type\": \"{}\", \"hash\": \"{}\"}}", event, hash_as_str)).expect("Unable to build JSON log event");

            debug!("Json string: {}", json);
            
            let mut resp_buf = [0x00;128];

            if let Ok(req)  = http_client.request(Method::POST, &url).await {
                if let Ok(_) = req.content_type(reqwless::headers::ContentType::ApplicationJson)
                    .body(json.as_bytes()).send(&mut resp_buf).await {
                    debug!("Log event POSTed successfully");
                }
                else {
                    error!("Unable to log event");
                }
            };

       }
    }
}