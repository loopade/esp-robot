use core::convert::TryInto;

use embedded_svc::wifi::{
    AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration, Protocol,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::EspDefaultNvsPartition,
    timer::EspTaskTimerService,
    wifi::{AsyncWifi, EspWifi, WifiEvent},
};

const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASS");
const FALLBACK_SSID: &str = "tracked-chassis";
const FALLBACK_PASSWORD: &str = env!("WIFI_FALLBACK_PASS");

pub async fn connect(
    modem: esp_idf_svc::hal::modem::Modem,
) -> anyhow::Result<AsyncWifi<EspWifi<'static>>> {
    return esp_common::connect_wifi(modem, FALLBACK_SSID).await;
    let sys_loop = EspSystemEventLoop::take()?;
    let _wifi_events = sys_loop.subscribe::<WifiEvent, _>(|event| {
        if let WifiEvent::StaDisconnected(info) = event {
            log::error!(
                "Wi-Fi disconnected: reason={}, rssi={}, bssid={:02x?}",
                info.reason(),
                info.rssi(),
                info.bssid()
            );
        }
    })?;
    let timer_service = EspTaskTimerService::new()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let mut wifi = AsyncWifi::wrap(
        EspWifi::new(modem, sys_loop.clone(), Some(nvs))?,
        sys_loop,
        timer_service,
    )?;

    let ssid = SSID
        .try_into()
        .map_err(|_| anyhow::anyhow!("WIFI_SSID is too long"))?;
    let password = PASSWORD
        .try_into()
        .map_err(|_| anyhow::anyhow!("WIFI_PASS is too long"))?;
    wifi.set_configuration(&Configuration::Mixed(
        ClientConfiguration {
            ssid,
            password,
            auth_method: AuthMethod::WPA2Personal,
            ..Default::default()
        },
        AccessPointConfiguration {
            ssid: FALLBACK_SSID.try_into().unwrap(),
            password: FALLBACK_PASSWORD.try_into().unwrap(),
            auth_method: AuthMethod::WPA2Personal,
            channel: 1,
            protocols: Protocol::P802D11B | Protocol::P802D11BG | Protocol::P802D11BGN,
            max_connections: 4,
            ..Default::default()
        },
    ))?;
    wifi.start().await?;
    log::info!(
        "Wi-Fi fallback AP ready: ssid={FALLBACK_SSID}, IP={:?}",
        wifi.wifi().ap_netif().get_ip_info()?
    );

    log::info!("Connecting STA to {SSID} with WPA2-PSK");
    if let Err(err) = wifi.connect().await {
        log::error!("Upstream Wi-Fi unavailable: {err}; fallback AP remains active");
        return Ok(wifi);
    }
    log::info!("Wi-Fi associated");
    if let Err(err) = wifi.wait_netif_up().await {
        log::error!("Upstream DHCP unavailable: {err}; fallback AP remains active");
        return Ok(wifi);
    }
    log::info!(
        "Upstream Wi-Fi ready: {:?}",
        wifi.wifi().sta_netif().get_ip_info()?
    );
    Ok(wifi)
}
