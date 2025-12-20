use embassy_time::Duration;

pub(crate) enum LatchMode {
    Latching, //Device/controller will remain enabled until another card is scanned to disable it
    Timed(Duration), //Device controller will remain enabled for <time> then disable again
}

pub(crate) struct Config<'a> {
    pub ssid: &'a str,
    pub wifi_pw: &'a str,
    pub device_name: &'a str,
    pub url_endpoint: &'a str,
    pub db_prefix: &'a str,
    pub db_version_prefix: &'a str,
    pub log_prefix: &'a str,
    pub http_timeout: Duration,
    pub latch_mode: LatchMode,
    pub db_sync_frequency: Duration,
}

pub(crate) static CONFIG: Config = Config {
    ssid: "YOUR_SSID",
    wifi_pw: "YOUR_WIFI_PW",
    device_name: "DEVICE_NAME",
    url_endpoint: "http://YOUR_URL_ENDPOINT",
    db_prefix: "db",
    db_version_prefix: "dbVersion",
    log_prefix: "logEvent",
    http_timeout: Duration::from_secs(10),
    latch_mode: LatchMode::Timed(Duration::from_secs(5)),
    db_sync_frequency: Duration::from_secs(5 * 60),
};
