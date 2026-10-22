use esp_idf_svc::wifi::{AsyncWifi, EspWifi};

pub async fn connect(
    modem: esp_idf_svc::hal::modem::Modem,
) -> anyhow::Result<AsyncWifi<EspWifi<'static>>> {
    esp_common::connect_wifi(modem).await
}
