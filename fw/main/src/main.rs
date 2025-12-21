#![no_std]
#![no_main]
#![allow(async_fn_in_trait)]

use cyw43::JoinOptions;
use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};

use defmt::*;

use {defmt_rtt as _, panic_probe as _};

use assign_resources::assign_resources;

use embassy_executor::Spawner;
use embassy_net::{Config as WifiConfig, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals;
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_time::Timer;

use static_cell::StaticCell;

use rand::RngCore;

mod database_task;
mod local_cardreader_task;
mod log_task;
mod main_task;
mod remote_cardreader_task;
mod watchdog;

use database_task::database_task;
use local_cardreader_task::local_cardreader_task;
use main_task::main_task;
use remote_cardreader_task::remote_cardreader_task;
use watchdog::watchdog_task;

use log_task::{log_task, LogEvent, LOG_EVENT_QUEUE};
mod config;
use config::CONFIG;

assign_resources! {
    //Status LEDs
    status_leds: StatusLedResources {
        red_led: PIN_7,
        green_led: PIN_8,
    },
    relay: RelayResources {
        relay_pin: PIN_15,
    }
    //SPI1 bus + WP/Hold pins for comms with the flash IC
    flash: FlashResources {
        spi: SPI1,
        sck: PIN_10,
        mosi: PIN_11,
        miso: PIN_12,
        cs: PIN_13,
        wp: PIN_14,
        hold: PIN_9,
    },
    //UART is used for RS485 link to anoter cardreader unit (if feature selected)
    uart: UartResources {
        tx: PIN_0,
        rx: PIN_1,
        uart: UART0,
        tx_dma: DMA_CH2,
        rx_dma: DMA_CH3,
    },
    watchdog: WatchdogResources {
        dog: WATCHDOG,
        heartbeat_led: PIN_6,
    },
    spi0: Spi0Resources {
        spi: SPI0,
        sck: PIN_18,
        mosi: PIN_19,
        miso: PIN_16,
        cs: PIN_17,
        rst: PIN_21,
    },
    wifi: WifiResources {
        pwr: PIN_23,
        cs: PIN_25,
        pio: PIO0,
        dma_ch: DMA_CH0,
        pin_24: PIN_24,
        pin_29: PIN_29,
    }
}

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let resources = split_resources!(p);

    //Spawn the watchdog task
    spawner.must_spawn(watchdog_task(resources.watchdog));

    let pwr = Output::new(resources.wifi.pwr, Level::Low);
    let cs = Output::new(resources.wifi.cs, Level::High);
    let mut pio = Pio::new(resources.wifi.pio, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        resources.wifi.pin_24,
        resources.wifi.pin_29,
        resources.wifi.dma_ch,
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

    //Spawn the appropriate runner task for local (direct SPI) or remote (via RS485 link) cardreader
    if cfg!(not(feature = "remote-cardreader")) {
        info!("Local cardreader mode selected");
        info!("NB - if no cardreader, this will hang and reboot repeatedly (by watchdog)");
        //Local task - will poll SPI cardreader over local bus
        spawner.must_spawn(local_cardreader_task(resources.spi0));
    } else {
        debug!("Spawning remote card reader task");
        spawner.must_spawn(remote_cardreader_task(resources.uart));
    }

    //Spawn the main task
    spawner.must_spawn(main_task(resources.status_leds, resources.relay));

    //Spawn the database task (2mbit flash, start addr 0)
    spawner.must_spawn(database_task(resources.flash, 0x00, stack));

    //Spawn the logger task
    spawner.must_spawn(log_task(stack));

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
