use embassy_net::{
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
    Stack,
};
use embassy_rp::clocks::RoscRng;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Timer, WithTimeout};

use defmt::*;

use rand::RngCore;

use reqwless::client::{HttpClient, TlsConfig, TlsVerify};
use reqwless::request::Method;
use reqwless::{request::RequestBuilder, response::StatusCode};

use crate::CONFIG;

const MAX_QUEUE_LEN: usize = 32usize;

pub(crate) enum LogEvent {
    ACTIVATED([u8; 32]),
    DEACTIVATED([u8; 32]),
    LOGINFAIL([u8; 32]),
    ERROR,
}

//The queue can hold 32 events awaiting logging
pub static LOG_EVENT_QUEUE: Channel<ThreadModeRawMutex, LogEvent, MAX_QUEUE_LEN> =
    Channel::<ThreadModeRawMutex, LogEvent, MAX_QUEUE_LEN>::new();



pub enum LogError {
    WifiNotConnected,
    ConnectionError, //Reqwless unable to connect
    Timeout,
    RemoteServerError(reqwless::response::StatusCode), //Http error from remote server (not 200!)
}

impl From<reqwless::Error> for LogError {
    fn from(_err: reqwless::Error) -> Self {
        Self::ConnectionError
    }
}

pub struct LogTaskRunner {
    stack: Stack<'static>,
}

impl LogTaskRunner {
    pub fn new(stack: Stack<'static>) -> Self {
        Self { stack }
    }

    pub async fn run(self) -> ! {
        loop {
            //Await an event from the queue
            let event = LOG_EVENT_QUEUE.receive().await;
            match self.log_event(&event).await {
                Ok(_) => {
                    info!("Log event recorded successfully")
                }
                Err(_) => {
                    warn!("Log event failed, will be requeued");
                    //Attempt to requeue
                    if let Err(_e) = LOG_EVENT_QUEUE.try_send(event) {
                        error!("Unable to requeue - this event will be lost")
                    }
                    //Don't try to log again for another minute after a failed attempt
                    Timer::after_secs(60).await;
                }
            }
        }
    }

    async fn log_event(&self, event: &LogEvent) -> Result<(), LogError> {
        //Abandon if wifi not running
        while !self.stack.is_config_up() {
            warn!("Log event failed, wifi not yet up");
            return Err(LogError::WifiNotConnected);
        }

        //Convert hash to ascii string representation
        let hash = match event {
            LogEvent::ACTIVATED(hash) | LogEvent::DEACTIVATED(hash) | LogEvent::LOGINFAIL(hash) => {
                //Convert hash to an ascii str representation
                hash
            }
            _ => b"N/A                             ", //32 bytes long too...
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
        debug!("Connecting to log endpoint");
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
        .expect("Unable to build JSON string event");

        debug!("Json string: {}", json);
        let mut rx_buf = [0x00; 512];

        let request = match http_client.request(Method::POST, &url).with_timeout(CONFIG.http_timeout)
        .await
        {
            Ok(e) => e?,
            Err(_timeout) => {
                error!("Log attempt failed (request timeout)");
                return Err(LogError::Timeout);
            }
        };

        debug!("Connecting");
        let retval = match request.content_type(reqwless::headers::ContentType::ApplicationJson)
            .body(json.as_bytes())
            .send(&mut rx_buf).with_timeout(CONFIG.http_timeout)
            .await
        {
            Ok(result) => {
                match result {
                    Ok(response) => {
                        //Valid HTTP response received - check it is 'ok'
                        if StatusCode::is_successful(&response.status) {
                            //Success!
                            Ok(())
                        }
                        else {
                            //Successfully connected, but didn't get 200 OK (or another success code)
                            Err(LogError::RemoteServerError(response.status))
                        }
                    },
                    Err(_error) => {
                        Err(LogError::ConnectionError)
                    },
                }
            },
            Err(_timeout) => {
                error!("Log attempt failed (send timeout)");
                Err(LogError::Timeout)            
            }
        };
        retval //Borrow checker gets unhappy otherwise!
    }
}
