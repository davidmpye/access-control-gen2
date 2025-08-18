use crate::W25q32jv;
use embassy_rp::{peripherals::SPI1, spi::Blocking, spi::Spi};
use embedded_io_async::Read;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use defmt::*;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::Timer;

use embassy_net::Stack;


use ekv::flash::{self, PageID, Flash};
use ekv::{config, Database};
use reqwless::response::StatusCode;


use crate::{ENDPOINT_URL, DEVICE_NAME};


use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};

use reqwless::client::{HttpClient, TlsConfig, TlsVerify};
use reqwless::request::Method;
use embassy_rp::clocks::RoscRng;
use rand::RngCore;

// Workaround for alignment requirements.
#[repr(C, align(4))]
struct AlignedBuf<const N: usize>([u8; N]);

struct DbFlash<T: NorFlash + ReadNorFlash> {
    start: usize,
    flash: T,
}

enum UpdateError {
    ConnectionError,                                    //Reqwless unable to connect
    RemoteServerError(reqwless::response::StatusCode),  //Http error from remote server (not 200!)
    InvalidDbVersion,                                   //DBVersion should (currently) be a 16 byte MD5 hash
}

impl From<reqwless::Error> for UpdateError {
    fn from(_err: reqwless::Error) -> Self {
        Self::ConnectionError
    }
}

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

impl <T>DatabaseRunner<T> where T: NorFlash+ ReadNorFlash {
    pub fn new(flash: T, size:usize, start_addr: usize, stack: Stack<'static>) -> Self {
        Self { flash, size, start_addr, stack}
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
        }
        else {
            info!("Local hash database mounted OK");
        }

        //Bring up the wifi stack
        info!("DHCP init");
        while !self.stack.is_config_up() {
            Timer::after_millis(100).await;
        }
        info!("DHCP ready, link init");
        while !self.stack.is_link_up() {
            Timer::after_millis(500).await;
        }
        info!("Link ready, awaiting stack up");
        self.stack.wait_config_up().await;
        info!("Stack ready");

        info!("Reqwless HTTP client init");
        let mut tls_read_buffer = [0; 16640];
        let mut tls_write_buffer = [0; 16640];
        let mut rng = RoscRng;
        let seed = rng.next_u64();

        let client_state = TcpClientState::<1, 1024, 1024>::new();
        let tcp_client = TcpClient::new(self.stack, &client_state);
        let dns_client = DnsSocket::new(self.stack);
        let tls_config = TlsConfig::new(
            seed,
            &mut tls_read_buffer,
            &mut tls_write_buffer,
            TlsVerify::None,
        );
        let mut client= HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);

        sync_database(&db, &mut client, "noop").await;
        loop {
            //Fixme 
            Timer::after_millis(500).await;
        }
    }
}

async fn sync_database<T: NorFlash + ReadNorFlash>(db: &Database<DbFlash<T>, NoopRawMutex>, client: &mut HttpClient<'_, TcpClient<'_, 1>, DnsSocket<'_>>, url:&str) {
    //Check current database version
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; 32];

    let current_db_version = rtx.read(b"__DB_VERSION__", &mut buf).await.map(|n| &buf[..n]).ok().expect("Fatal error - unable to read database version");
    info!("Current database version: {:a}", current_db_version);
    drop(rtx);

    match get_remote_db_version(client).await {
        Ok(remote_db_version) => {
            info!("Remote DB version is {}", remote_db_version);
            if remote_db_version == current_db_version {
                info!("No update needed - database in sync");
            }
            else {
                info!("Preparing to download new database");
                //need to actually do it!
            }
        }
        Err(e) =>  {
            error!("Unable to get remote DB version")
        }
    }
}



async fn get_remote_db_version(http_client: &mut HttpClient<'_, TcpClient<'_, 1>, DnsSocket<'_>>) ->Result<[u8;16], UpdateError> {

    let mut url_buf = [0x00u8;128];
    let url = format_no_std::show(&mut url_buf, format_args!("{}/{}/dbVersion", ENDPOINT_URL, DEVICE_NAME)).expect("Unable to build DB update URL");
    info!("Connecting to {}", &url);

    //Make connection
    let mut rx_buffer = [0; 8192];
    let mut request = http_client.request(Method::GET, &url).await?;
    let response = request.send(&mut rx_buffer).await?;

    if ! StatusCode::is_successful(&response.status) {
        error!("Http connection error: {}", &response.status);
        return Err(UpdateError::RemoteServerError(response.status));
    }
        
    info!("Connection successful");
    //Read 100 bytes
    let mut buf = [0x00u8; 100];
    let mut reader = response.body().reader();
    let len = reader.read(&mut buf).await?;
        
    if len != 16 {
        error!("Wrong DBVersion length - should be 16 bytes, got {}", len);
        return Err(UpdateError::InvalidDbVersion)
    }

    let mut res = [0x00u8;16];
    res.copy_from_slice(&rx_buffer[0..16]);
    return Ok(res);    
}