use core::convert::TryInto;

use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition, timer::EspTaskTimerService, wifi::{AsyncWifi, EspWifi, WifiEvent}};

const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASS");

pub async fn connect(
    modem: esp_idf_svc::hal::modem::Modem,
) -> anyhow::Result<AsyncWifi<EspWifi<'static>>> {
    return esp_common::connect_wifi(modem, "tracked-arm").await;
    let sys_loop = EspSystemEventLoop::take()?;
    let _events = sys_loop.subscribe::<WifiEvent, _>(|event| {
        if let WifiEvent::StaDisconnected(info) = event { log::warn!("wifi disconnected: reason={}", info.reason()); }
    })?;
    let nvs = EspDefaultNvsPartition::take()?;
    let timer = EspTaskTimerService::new()?;
    let mut wifi = AsyncWifi::wrap(EspWifi::new(modem, sys_loop.clone(), Some(nvs))?, sys_loop, timer)?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration { ssid: SSID.try_into().unwrap(), password: PASSWORD.try_into().unwrap(), auth_method: AuthMethod::WPA2Personal, ..Default::default() }))?;
    wifi.start().await?;
    wifi.connect().await?;
    wifi.wait_netif_up().await?;
    log::info!("wifi ready: {:?}", wifi.wifi().sta_netif().get_ip_info()?);
    Ok(wifi)
}
