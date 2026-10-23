use esp_idf_svc::{nvs::EspDefaultNvsPartition, wifi::{AsyncWifi, EspWifi}};

pub async fn connect(
    modem: esp_idf_svc::hal::modem::Modem,
    nvs: EspDefaultNvsPartition,
) -> anyhow::Result<AsyncWifi<EspWifi<'static>>> {
    esp_common::connect_wifi(modem, nvs).await
}
