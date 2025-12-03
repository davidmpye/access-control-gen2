#![no_std]
#![no_main]
#![allow(async_fn_in_trait)]

use cyw43::JoinOptions;
use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};

//For SPI flash
use w25q32jv::W25q32jv;

use defmt::*;

use {defmt_rtt as _, panic_probe as _};

use embassy_executor::Spawner;
use embassy_net::{Config as WifiConfig, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0, UART0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::spi::{Config as SpiConfig, Spi};
use embassy_rp::uart::{Config as UartConfig, InterruptHandler as UartInterruptHandler, Uart};
use embassy_time::{Delay, Timer};

use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};

use static_cell::StaticCell;

use rand::RngCore;

mod database_task;
mod local_cardreader_task;
mod log_task;
mod main_task;
mod remote_cardreader_task;
mod watchdog;

use database_task::DatabaseRunner;
use local_cardreader_task::LocalCardreaderTaskRunner;
use main_task::main_task;
use remote_cardreader_task::remote_cardreader_task;
use watchdog::watchdog_task;

use log_task::{LogEvent, LogTaskRunner, LOG_EVENT_QUEUE};
mod config;
use config::CONFIG;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    UART0_IRQ => UartInterruptHandler<UART0>;
});

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

#[embassy_executor::task]
async fn database_task(
    runner: DatabaseRunner<
        W25q32jv<
            ExclusiveDevice<
                Spi<'static, embassy_rp::peripherals::SPI1, embassy_rp::spi::Blocking>,
                Output<'static>,
                NoDelay,
            >,
            Output<'static>,
            Output<'static>,
        >,
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn log_task(runner: LogTaskRunner) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn local_cardreader_task(
    mut runner: LocalCardreaderTaskRunner<
        ExclusiveDevice<
            Spi<'static, embassy_rp::peripherals::SPI0, embassy_rp::spi::Blocking>,
            Output<'static>,
            Delay,
        >,
        Output<'static>,
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    //Spawn the watchdog task
    spawner.must_spawn(watchdog_task(p.WATCHDOG, Output::new(p.PIN_6, Level::High)));

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

    //Set up SPI1 for the flash memory storage
    let (sck, mosi, miso, cs) = (p.PIN_10, p.PIN_11, p.PIN_12, &p.PIN_13);
    let spi1: Spi<'_, embassy_rp::peripherals::SPI1, embassy_rp::spi::Blocking> =
        Spi::new_blocking(p.SPI1, sck, mosi, miso, SpiConfig::default());
    let flash_wp = Output::new(p.PIN_14, Level::Low); //WP is ACTIVE LOW - start with flash WP set
    let flash_hold = Output::new(p.PIN_9, Level::High); //Flash hold is ACTIVE LOW - start with hold not enabled
    let flash_cs = Output::new(p.PIN_13, Level::High); //SPI flash CS pin
    let spi_device = embedded_hal_bus::spi::ExclusiveDevice::new_no_delay(spi1, flash_cs);
    let mut spi_flash =
        W25q32jv::new(spi_device, flash_hold, flash_wp).expect("Unable to initialise flash");
    info!(
        "SPI flash (W25Q32) initialised - device id {}",
        spi_flash.device_id().expect("Unable to read flash ID")
    );

    //Wifi hardware setup
    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    unwrap!(spawner.spawn(cyw43_task(runner)));

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    // Init network stack
    let config = WifiConfig::dhcpv4(Default::default());
    let mut rng = RoscRng;
    let seed = rng.next_u64();
    static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init(StackResources::new()),
        seed,
    );
    //Spawn network task
    unwrap!(spawner.spawn(net_task(runner)));

    //Set up channel to receive card hash
    //Set up the appropriate task to read from the card reader - either local (direct SPI) or remote (via RS485 link)
    if cfg!(not(feature = "remote-cardreader")) {
        info!("Local cardreader mode selected");
        info!("NB - if no cardreader, this will hang and reboot repeatedly (by watchdog)");
        //Local task - will poll SPI cardreader over local bus
        let (sck, mosi, miso, cs) = (p.PIN_18, p.PIN_19, p.PIN_16, p.PIN_17);
        let spi0 = Spi::new_blocking(p.SPI0, sck, mosi, miso, SpiConfig::default());
        let spi0: ExclusiveDevice<
            Spi<'_, embassy_rp::peripherals::SPI0, embassy_rp::spi::Blocking>,
            Output,
            Delay,
        > = ExclusiveDevice::new(spi0, Output::new(cs, Level::High), Delay);
        debug!("Spawning local card reader task");
        spawner.must_spawn(local_cardreader_task(LocalCardreaderTaskRunner::new(
            spi0,
            Output::new(p.PIN_21, Level::High),
        )));
    } else {
        //Remote task
        info!("Remote cardreader mode selected");
        let (tx_pin, rx_pin, uart) = (p.PIN_0, p.PIN_1, p.UART0);
        let uart = Uart::new(
            uart,
            tx_pin,
            rx_pin,
            Irqs,
            p.DMA_CH2,
            p.DMA_CH3,
            UartConfig::default(),
        );
        debug!("Spawning remote card reader task");
        spawner.must_spawn(remote_cardreader_task(uart));
    }

    //Spawn the main task
    let allowed = Output::new(p.PIN_7, Level::Low);
    let denied = Output::new(p.PIN_8, Level::Low);
    let relay_pin = Output::new(p.PIN_15, Level::Low);
    spawner.must_spawn(main_task(relay_pin, allowed, denied));

    //Spawn the database task
    spawner.must_spawn(database_task(DatabaseRunner::new(
        spi_flash,
        2 * 1024 * 1024, //2mbit
        0x00,
        stack,
    )));

    //Spawn the logger task
    spawner.must_spawn(log_task(LogTaskRunner::new(stack)));

    loop {
        match control
            .join(CONFIG.ssid, JoinOptions::new(CONFIG.wifi_pw.as_bytes()))
            .await
        {
            Ok(_) => {
                info!("WiFi network {} joined, configuring stack", CONFIG.ssid);
                break;
            }
            Err(err) => {
                error!(
                    "Failed to join {}, status {}, retrying in 10s",
                    CONFIG.ssid, err.status
                );
                Timer::after_secs(10).await;
            }
        }
    }

    //Complete init of Wifi stack
    debug!("DHCP init");
    stack.wait_config_up().await;
    debug!("Config ready, awaiting link up");
    stack.wait_link_up().await;
    debug!("Link ready, awaiting config up");
    stack.wait_config_up().await;
    info!("Wifi ready");
    //Main function is now complete - the peripherals/tasks/stack are operational
}
