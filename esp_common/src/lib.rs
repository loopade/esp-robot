use core::convert::TryInto;
use embedded_svc::wifi::{AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration, Protocol};
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition, timer::EspTaskTimerService, wifi::{AsyncWifi, EspWifi}};

pub async fn connect_wifi(modem: esp_idf_svc::hal::modem::Modem, fallback_ssid: &'static str) -> anyhow::Result<AsyncWifi<EspWifi<'static>>> {
    let events = EspSystemEventLoop::take()?;
    let timer = EspTaskTimerService::new()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let mut wifi = AsyncWifi::wrap(EspWifi::new(modem, events.clone(), Some(nvs))?, events, timer)?;
    let ssid: heapless::String<32> = env!("WIFI_SSID").try_into().map_err(|_| anyhow::anyhow!("WIFI_SSID too long"))?;
    let password: heapless::String<64> = env!("WIFI_PASS").try_into().map_err(|_| anyhow::anyhow!("WIFI_PASS too long"))?;
    let ap_password: heapless::String<64> = option_env!("WIFI_FALLBACK_PASS").unwrap_or("<set-locally>").try_into().map_err(|_| anyhow::anyhow!("WIFI_FALLBACK_PASS too long"))?;
    wifi.set_configuration(&Configuration::Mixed(ClientConfiguration { ssid, password, auth_method: AuthMethod::WPA2Personal, ..Default::default() }, AccessPointConfiguration { ssid: fallback_ssid.try_into().unwrap(), password: ap_password, auth_method: AuthMethod::WPA2Personal, channel: 1, protocols: Protocol::P802D11B | Protocol::P802D11BG | Protocol::P802D11BGN, max_connections: 4, ..Default::default() }))?;
    wifi.start().await?;
    if let Err(error) = wifi.connect().await { log::warn!("STA connection failed: {error}; AP remains available"); } else { wifi.wait_netif_up().await?; }
    Ok(wifi)
}
