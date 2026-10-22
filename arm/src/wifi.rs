use esp_idf_svc::{nvs::EspDefaultNvsPartition, wifi::{AsyncWifi, EspWifi}};

pub async fn connect(
    modem: esp_idf_svc::hal::modem::Modem,
) -> anyhow::Result<(AsyncWifi<EspWifi<'static>>, EspDefaultNvsPartition)> {
    esp_common::connect_wifi_with_nvs(modem).await
}
