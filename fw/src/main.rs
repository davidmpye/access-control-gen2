#![no_std]
#![no_main]
#![allow(async_fn_in_trait)]

use core::env;
use cyw43::JoinOptions;
use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};
use defmt::*;
use embedded_io_async::Read;

use embassy_executor::Spawner;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_net::{Config, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{UART0, WATCHDOG, DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_time::{Duration, Timer, Delay};
use embassy_rp::uart::{Uart, Config as UartConfig, InterruptHandler as UartInterruptHandler, Async};
use embassy_rp::spi::{Config as SpiConfig, Spi};



use reqwless::client::{HttpClient, TlsConfig, TlsVerify};
use reqwless::request::Method;
use static_cell::StaticCell;
use crate::remote_cardreader::remote_cardreader_task;

use {defmt_rtt as _, panic_probe as _};


use embedded_hal_bus::spi::ExclusiveDevice;
use rand::RngCore;

mod remote_cardreader;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    UART0_IRQ => UartInterruptHandler<UART0>;
});

const WIFI_NETWORK: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PW");

const ENDPOINT_URL:&str = "https://www.bbc.co.uk:443";

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {

    let p = embassy_rp::init(Default::default());
    let mut rng = RoscRng;

    let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");
    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs download 43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs download 43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
    //let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    //let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    unwrap!(spawner.spawn(cyw43_task(runner)));

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let config = Config::dhcpv4(Default::default());

    // Generate random seed
    let seed = rng.next_u64();

    // Init network stack
    static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    let (_stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init(StackResources::new()),
        seed,
    );
    //Spawn network task
    unwrap!(spawner.spawn(net_task(runner)));

    //Set up SPI1 for the flash memory storage
    let (sck, mosi, miso, cs) = (p.PIN_10, p.PIN_11, p.PIN_12, p.PIN_13);
    let spi1 = Spi::new_blocking(p.SPI1, sck, mosi, miso, SpiConfig::default());
    //NB - also need to set FLASH_WP and FLASH_HOLD - these probably don't need to be on GPIOs, and could just be 
    //permanently set, but for now, they are on GPIOs.
    let flash_wp = Output::new(p.PIN_14, Level::Low);  //WP is ACTIVE LOW - start with flash WP set
    let flash_hold = Output::new(p.PIN_9, Level::High); //Flash hold is ACTIVE LOW - start with hold not enabled
    

    //Configure the relay driver MOSFET Gate pin
    let mosfet_pin = Output::new(p.PIN_15, Level::Low);

    //Set up channel to receive card hash 
    //Set up the appropriate task to read from the card reader - either local (direct SPI) or remote (via RS485 link)
    
    if cfg!(not(feature = "remote-cardreader")) {   
        //Local task - will poll SPI cardreader over local bus
        let (sck, mosi, miso, cs) = ( p.PIN_18, p.PIN_19, p.PIN_16, p.PIN_17);
        let spi0 = Spi::new_blocking(p.SPI0, sck, mosi, miso, SpiConfig::default());
        let mut spi0 = ExclusiveDevice::new(spi0, cs, Delay);
        //spawner.must_spawn(local_cardreader_task(spi));
    }
    else {  
        //Remote task
        let (tx_pin, rx_pin, uart) = (p.PIN_0, p.PIN_1, p.UART0);
        let uart = Uart::new(uart, tx_pin, rx_pin, Irqs, p.DMA_CH2, p.DMA_CH3, UartConfig::default());
        spawner.must_spawn(remote_cardreader_task(uart));
    }
/*
    loop {
        match control
            .join(WIFI_NETWORK, JoinOptions::new(WIFI_PASSWORD.as_bytes()))
            .await
        {
            Ok(_) => break,
            Err(err) => {
                info!("join failed with status={}", err.status);
            }
        }
    }

    info!("waiting for DHCP...");
    while !stack.is_config_up() {
        Timer::after_millis(100).await;
    }
    info!("DHCP is now up!");

    info!("waiting for link up...");
    while !stack.is_link_up() {
        Timer::after_millis(500).await;
    }
    info!("Link is up!");

    info!("waiting for stack to be up...");
    stack.wait_config_up().await;
    info!("Stack is up!");


 */




}
    // And now we can use it!
/* 
    loop {
        let mut rx_buffer = [0; 8192];
        let mut tls_read_buffer = [0; 16640];
        let mut tls_write_buffer = [0; 16640];

        let client_state = TcpClientState::<1, 1024, 1024>::new();
        let tcp_client = TcpClient::new(stack, &client_state);
        let dns_client = DnsSocket::new(stack);
        let tls_config = TlsConfig::new(
            seed,
            &mut tls_read_buffer,
            &mut tls_write_buffer,
            TlsVerify::None,
        );

        let mut http_client = HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);
       
        info!("connecting to {}", &ENDPOINT_URL);

        let mut request = match http_client.request(Method::GET, &ENDPOINT_URL).await {
            Ok(req) => req,
            Err(e) => {
                error!("Failed to make HTTP request: {:?}", e);
                continue;
            }
        };

        let response = match request.send(&mut rx_buffer).await {
            Ok(resp) => resp,
            Err(_e) => {
                error!("Failed to send HTTP request");
                continue;
            }
        };

        //Read 100 bytes
        let mut buf = [0x00u8; 100];

        let mut reader = response.body().reader();

        while let Ok(len) = reader.read(&mut buf).await {
            if len == 0 {
                break;
            }
            info!("Got {}", buf[0..len])
        }

        Timer::after(Duration::from_secs(5)).await;
    }
    */

