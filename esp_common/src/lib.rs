use core::convert::TryInto;
use embassy_time::{Duration, Timer};
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition, timer::EspTaskTimerService, wifi::{AsyncWifi, EspWifi}};

pub async fn connect_wifi(modem: esp_idf_svc::hal::modem::Modem, nvs: EspDefaultNvsPartition) -> anyhow::Result<AsyncWifi<EspWifi<'static>>> {
    Ok(connect_wifi_inner(modem, nvs).await?)
}

async fn connect_wifi_inner(modem: esp_idf_svc::hal::modem::Modem, nvs: EspDefaultNvsPartition) -> anyhow::Result<AsyncWifi<EspWifi<'static>>> {
    let events = EspSystemEventLoop::take()?;
    let timer = EspTaskTimerService::new()?;
    let mut wifi = AsyncWifi::wrap(EspWifi::new(modem, events.clone(), Some(nvs))?, events, timer)?;
    let ssid: heapless::String<32> = env!("WIFI_SSID").try_into().map_err(|_| anyhow::anyhow!("WIFI_SSID too long"))?;
    let password: heapless::String<64> = env!("WIFI_PASS").try_into().map_err(|_| anyhow::anyhow!("WIFI_PASS too long"))?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration { ssid, password, auth_method: AuthMethod::WPA2Personal, ..Default::default() }))?;
    wifi.start().await?;
    loop {
        match wifi.connect().await {
            Ok(()) => match wifi.wait_netif_up().await {
                Ok(()) => {
                    log::info!("Wi-Fi connected: {:?}", wifi.wifi().sta_netif().get_ip_info()?);
                    break;
                }
                Err(error) => {
                    log::warn!("STA got no network address: {error}; retrying in 2 seconds");
                }
            },
            Err(error) => {
                log::warn!("STA connection failed: {error}; retrying in 2 seconds");
            }
        }
        let _ = wifi.disconnect().await;
        Timer::after(Duration::from_secs(2)).await;
    }
    Ok(wifi)
}
