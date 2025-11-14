use defmt::*;

use embedded_io_async::Read;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};

use embassy_net::{
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
    Stack,
};
use embassy_rp::clocks::RoscRng;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;

use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::signal::Signal;

use embassy_time::{Duration, Instant, Timer};

use ekv::flash::{self, PageID};
use ekv::{config, Database};

use heapless::Vec;

use reqwless::client::{HttpClient, TlsConfig, TlsVerify};
use reqwless::request::Method;
use reqwless::response::StatusCode;

use rand::RngCore;

use crate::config::CONFIG;

// Workaround for alignment requirements.
#[repr(C, align(4))]
struct AlignedBuf<const N: usize>([u8; N]);

struct DbFlash<T: NorFlash + ReadNorFlash> {
    start: usize,
    flash: T,
}

pub enum UpdateError {
    WifiNotConnected,
    ConnectionError, //Reqwless unable to connect
    Timeout,
    RemoteServerError(reqwless::response::StatusCode), //Http error from remote server (not 200!)
    InvalidDbVersion, //DBVersion should (currently) be a 16 byte MD5 hash
}

impl From<reqwless::Error> for UpdateError {
    fn from(_err: reqwless::Error) -> Self {
        Self::ConnectionError
    }
}

pub enum DatabaseTaskCommand {
    CheckMD5Hash([u8; 32]),
    ForceUpdate,
}

pub enum DatabaseTaskResponse {
    Found,
    NotFound,
    Invalid,
    Error,
    UpdateOk,
}

pub static DATABASE_COMMAND_SIGNAL: Signal<ThreadModeRawMutex, DatabaseTaskCommand> = Signal::new();
pub static DATABASE_RESPONSE_SIGNAL: Signal<ThreadModeRawMutex, DatabaseTaskResponse> =
    Signal::new();

//This is an EKV<->NorFlash+ReadNorFlash shim
impl<T: NorFlash + ReadNorFlash> flash::Flash for DbFlash<T> {
    type Error = T::Error;
    fn page_count(&self) -> usize {
        config::MAX_PAGE_COUNT
    }

    async fn erase(&mut self, page_id: PageID) -> Result<(), <DbFlash<T> as flash::Flash>::Error> {
        self.flash.erase(
            (self.start + page_id.index() * config::PAGE_SIZE) as u32,
            (self.start + page_id.index() * config::PAGE_SIZE + config::PAGE_SIZE) as u32,
        )
    }

    async fn read(
        &mut self,
        page_id: PageID,
        offset: usize,
        data: &mut [u8],
    ) -> Result<(), <DbFlash<T> as flash::Flash>::Error> {
        let address = self.start + page_id.index() * config::PAGE_SIZE + offset;
        let mut buf = AlignedBuf([0; config::PAGE_SIZE]);
        self.flash.read(address as u32, &mut buf.0[..data.len()])?;
        data.copy_from_slice(&buf.0[..data.len()]);
        Ok(())
    }

    async fn write(
        &mut self,
        page_id: PageID,
        offset: usize,
        data: &[u8],
    ) -> Result<(), <DbFlash<T> as flash::Flash>::Error> {
        let address = self.start + page_id.index() * config::PAGE_SIZE + offset;
        let mut buf = AlignedBuf([0; config::PAGE_SIZE]);
        buf.0[..data.len()].copy_from_slice(data);
        self.flash.write(address as u32, &buf.0[..data.len()])
    }
}

pub struct DatabaseRunner<T: NorFlash + ReadNorFlash> {
    flash: T,
    size: usize,
    start_addr: usize,
    stack: Stack<'static>,
}

impl<T> DatabaseRunner<T>
where
    T: NorFlash + ReadNorFlash,
{
    pub fn new(flash: T, size: usize, start_addr: usize, stack: Stack<'static>) -> Self {
        Self {
            flash,
            size,
            start_addr,
            stack,
        }
    }

    pub async fn run(self) -> ! {
        let flash: DbFlash<T> = DbFlash {
            flash: self.flash,
            start: self.start_addr,
        };

        //Initialise and mount the EKV database
        let db = Database::<_, NoopRawMutex>::new(flash, ekv::Config::default());
        if db.mount().await.is_err() {
            info!("Formatting...");
            db.format().await.expect("Flash format failure");
            //write version key post format
            let mut wtx = db.write_transaction().await;
            wtx.write(b"__DB_VERSION__", b"0x00").await.unwrap();
            wtx.commit().await.unwrap();
        } else {
            let mut buf = [0u8; 32];
            let rtx = db.read_transaction().await;
            let key_read = rtx
                .read(b"__DB_VERSION__", &mut buf)
                .await
                .map(|n| &buf[..n]);
            drop(rtx); //to allow write below, if needed.

            let current_db_version = match key_read {
                Ok(val) => val,
                Err(_e) => {
                    error!(
                        "DB version tag missing from flash - writing tag as 0x00 to force update"
                    );
                    let mut wtx = db.write_transaction().await;
                    wtx.write(b"__DB_VERSION__", b"0x00").await.unwrap();
                    wtx.commit().await.unwrap();
                    &[0x00]
                }
            };

            let rtx = db.read_transaction().await;

            let mut cursor = rtx.read_all().await.expect("Cursor fail");
            let mut count = 0usize;

            let mut keybuf = [0x00u8; 32];
            let mut valbuf = [0x00u8; 32];

            while cursor.next(&mut keybuf, &mut valbuf).await.ok() != Some(None) {
                count += 1;
                //Without a brief 1 micro wait, the watchdog doesnt have a chance to run.....
                Timer::after_micros(1).await;
            }

            info!("Local database version: {}, containing {} RFID hashes", current_db_version, count - 1); //-1 to account for the __DB_VERSION__ key
        }

        let mut last_sync_attempt_time = Instant::MIN;

        loop {
            //Sync 60 seconds after startup (to let wifi come up) and at specified intervals
            if last_sync_attempt_time == Instant::MIN
                && Instant::now() > Instant::MIN + Duration::from_secs(60)
                || Instant::now() > last_sync_attempt_time + CONFIG.db_sync_frequency
            {
                last_sync_attempt_time = Instant::now();
                info!("Database sync due - attempting");
                if sync_database(&db, self.stack).await.is_ok() {
                    info!("Sync OK");
                } else {
                    error!("Database sync failed");
                }
            }
            info!("Now awaiting database command signal");
            //Purpose of timeout is to give us an opportunity to check, every 60s, if we need to do a DB update
            match embassy_time::with_timeout(
                Duration::from_secs(60),
                DATABASE_COMMAND_SIGNAL.wait(),
            )
            .await
            {
                Ok(cmd) => match cmd {
                    DatabaseTaskCommand::CheckMD5Hash(hash) => match db_lookup(&db, hash).await {
                        Some(_) => {
                            DATABASE_RESPONSE_SIGNAL.signal(DatabaseTaskResponse::Found);
                        }
                        None => {
                            DATABASE_RESPONSE_SIGNAL.signal(DatabaseTaskResponse::NotFound);
                        }
                    },
                    DatabaseTaskCommand::ForceUpdate => {
                        info!("Database force-update checking");
                        defmt::todo!("Force update not implemented");
                    }
                },
                Err(_) => {
                    info!("Timed out awaiting signal, will check if update ready");
                    //Timeout
                }
            }
        }
    }
}

async fn db_lookup<T: NorFlash + ReadNorFlash>(
    db: &Database<DbFlash<T>, NoopRawMutex>,
    hash: [u8; 32],
) -> Option<()> {
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; 32];

    if let Some(_key) = rtx.read(&hash, &mut buf).await.map(|n| &buf[..n]).ok() {
        debug!("Key {:?} found in database", hash);
        Some(())
    } else {
        debug!("Key {:?} NOT found in database", hash);
        None
    }
}

async fn sync_database<T: NorFlash + ReadNorFlash>(
    db: &Database<DbFlash<T>, NoopRawMutex>,
    stack: Stack<'static>,
) -> Result<(), UpdateError> {
    //Check if network is up, abort if not
    if !stack.is_config_up() {
        error!("Unable to sync - no wifi connection");
        return Err(UpdateError::WifiNotConnected);
    }

    //Build a fresh http client for each database update attempt
    info!("Reqwless HTTP client init");
    let mut tls_read_buffer = [0; 8096];
    let mut tls_write_buffer = [0; 8096];
    let mut rng = RoscRng;
    let seed = rng.next_u64();

    let client_state = TcpClientState::<2, 1024, 1024>::new();
    let tcp_client = TcpClient::new(stack, &client_state);
    let dns_client = DnsSocket::new(stack);
    let tls_config = TlsConfig::new(
        seed,
        &mut tls_read_buffer,
        &mut tls_write_buffer,
        TlsVerify::None,
    );
    let mut http_client = HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);

    //Check current database version
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; 32];

    let current_db_version = rtx
        .read(b"__DB_VERSION__", &mut buf)
        .await
        .map(|n| &buf[..n])
        .ok()
        .expect("Fatal error - unable to read database version");
    info!("Current database version: {:a}", current_db_version);
    drop(rtx);

    match get_remote_db_version(&mut http_client).await {
        Ok(remote_db_version) => {
            info!("Remote DB version is {}", remote_db_version);
            if remote_db_version == current_db_version {
                info!("No update needed - database in sync");
            } else {
                info!(
                    "Commencing database update from {:a} to {:a}",
                    current_db_version, remote_db_version
                );
                //Erase existing database
                debug!("Erasing database");
                db.format().await.expect("Failed to erase database");
                debug!("Preparing to download new database");
                let mut url_buf = [0x00u8; 128];
                let url = format_no_std::show(
                    &mut url_buf,
                    format_args!(
                        "{}/{}/{}",
                        CONFIG.url_endpoint, CONFIG.device_name, CONFIG.db_prefix
                    ),
                )
                .expect("Unable to build DB update URL");
                debug!("Connecting to {}", &url);

                //Make connection
                let mut rx_buffer = [0; 2048];
                info!("Creating HTTP request");

                let mut request = match embassy_time::with_timeout(
                    CONFIG.http_timeout,
                    http_client.request(Method::GET, &url),
                )
                .await
                {
                    Ok(e) => e?,
                    Err(_) => {
                        error!("Timeout creating http request");
                        return Err(UpdateError::Timeout);
                    }
                };

                debug!("Connecting");
                let response = match embassy_time::with_timeout(
                    CONFIG.http_timeout,
                    request.send(&mut rx_buffer),
                )
                .await
                {
                    Ok(e) => e?,
                    Err(_) => {
                        error!("Database update failed (timed out)");
                        return Err(UpdateError::Timeout);
                    }
                };

                if !StatusCode::is_successful(&response.status) {
                    error!("Http connection error: {}", &response.status);
                    return Err(UpdateError::RemoteServerError(response.status));
                }

                debug!("Connected to server, receiving hashes");
                //We will process the hashes in 32 hash chunks, so we can sort and store them
                //Each hash is 32 bytes long with a trailing space as a separator.

                let mut buf = [0x00u8; 32 * 32 + 32]; //there will be a space between each
                let mut reader = response.body().reader();

                let mut buf_offset = 0usize;

                while let Ok(len) = reader.read(&mut buf[buf_offset..]).await {
                    if len == 0 {
                        //EOF
                        debug!("Hit EOF");
                        break;
                    }
                    debug!("Read {} bytes", len);
                    let mut store: Vec<[u8; 32], 32> = Vec::new();

                    let mut hashes = buf[..len + buf_offset].chunks_exact(33);
                    while let Some(hash) = hashes.next() {
                        //Convert hash into correct length type
                        let hash: [u8; 32] = hash[0..32].try_into().unwrap();
                        debug!(
                            "Storing hash {} into vec",
                            core::str::from_utf8(&hash).unwrap()
                        );
                        store.push(hash).expect("Heapless vec hash store error");
                    }

                    //Sort the store - ekv requires the keys to be sorted in order within a transaction
                    store.sort_unstable();

                    let mut wtx: ekv::WriteTransaction<'_, DbFlash<T>, NoopRawMutex> =
                        db.write_transaction().await;
                    for i in store {
                        debug!("Writing key: {:a}", i);
                        wtx.write(&i, &[0x00]).await.expect("Key write failure");
                    }
                    wtx.commit().await.expect("Transaction commit failed");

                    let excess_byte_count = (len + buf_offset) % 33;
                    if excess_byte_count != 0 {
                        //odd bytes, need to copy these to the start of the buffer, and set offset appropriately
                        //so the next read arrives at the right place and complete hash can be processed
                        let (left, right) =
                            buf.split_at_mut((len + buf_offset) - excess_byte_count);
                        debug!("Len is {}, excess byte count is {}", len, excess_byte_count);
                        debug!(
                            "Left is \n{}, right is \n{}",
                            core::str::from_utf8(left).unwrap(),
                            core::str::from_utf8(right).unwrap()
                        );
                        left[0..excess_byte_count].copy_from_slice(&right[..excess_byte_count]);
                        buf_offset = excess_byte_count;
                    } else {
                        buf_offset = 0;
                    }
                }
            }

            //Update the DB version tag
            let mut wtx = db.write_transaction().await;
            wtx.write(b"__DB_VERSION__", &remote_db_version)
                .await
                .unwrap();
            wtx.commit().await.unwrap();
            //Fixme  -remaining bytes
            info!("Database update completed successfully");
            Ok(())
        }
        Err(e) => {
            error!("Unable to get remote DB version");
            return Err(e);
        }
    }
}

async fn get_remote_db_version(
    http_client: &mut HttpClient<'_, TcpClient<'_, 2>, DnsSocket<'_>>,
) -> Result<[u8; 24], UpdateError> {
    let mut url_buf = [0x00u8; 128];
    let url = format_no_std::show(
        &mut url_buf,
        format_args!(
            "{}/{}/{}",
            CONFIG.url_endpoint, CONFIG.device_name, CONFIG.db_version_prefix
        ),
    )
    .expect("Unable to build DB update URL");
    debug!("Obtaining remote database version from {}", url);
    //Make connection
    let mut rx_buffer = [0; 2048];

    debug!("Creating HTTP request");
    let mut request = match embassy_time::with_timeout(
        CONFIG.http_timeout,
        http_client.request(Method::GET, &url),
    )
    .await
    {
        Ok(e) => e?,
        Err(_) => {
            error!("Timeout creating http request");
            return Err(UpdateError::Timeout);
        }
    };

    debug!("Connecting");
    let response =
        match embassy_time::with_timeout(CONFIG.http_timeout, request.send(&mut rx_buffer)).await {
            Ok(e) => e?,
            Err(_) => {
                error!("Timeout connecting to server {}", url);
                return Err(UpdateError::Timeout);
            }
        };

    if !StatusCode::is_successful(&response.status) {
        error!("Http server error: {}", &response.status);
        return Err(UpdateError::RemoteServerError(response.status));
    }

    debug!("Connection successful");
    //Read 100 bytes
    let mut buf = [0x00u8; 100];
    let mut reader = response.body().reader();
    let len = reader.read(&mut buf).await?;

    if len != 24 {
        error!("Wrong DBVersion length - should be 24 bytes, got {}", len);
        return Err(UpdateError::InvalidDbVersion);
    }

    let mut res = [0x00u8; 24];
    res.copy_from_slice(&rx_buffer[0..24]);
    return Ok(res);
}
