#[path = "../pca9685.rs"]
mod pca9685;

use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result};
use esp_idf_hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::units::FromValueType as _;

const PCA9685_ADDRESS: u8 = 0x40;
const PWM_FREQUENCY_HZ: f32 = 50.0;
const CHANNEL_COUNT: u8 = 16;
const STEP_DELAY: Duration = Duration::from_millis(250);
const HOLD_DELAY: Duration = Duration::from_secs(1);

fn set_all_channels(pwm: &mut pca9685::Pca9685<'_>, angle_deg: f32) -> Result<()> {
    for channel in 0..CHANNEL_COUNT {
        pwm.set_servo_angle(channel, angle_deg, 0.0, 180.0, 500.0, 2500.0)
            .with_context(|| format!("failed to set PCA9685 channel {channel}"))?;
        sleep(STEP_DELAY);
    }
    Ok(())
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("starting PCA9685 channel test");

    let peripherals = Peripherals::take().context("failed to take ESP32 peripherals")?;
    let i2c_config = I2cConfig::new().baudrate(100.kHz().into());
    let i2c = I2cDriver::new(
        peripherals.i2c0,
        peripherals.pins.gpio21,
        peripherals.pins.gpio22,
        &i2c_config,
    )
    .context("failed to initialize I2C0 on GPIO21/GPIO22")?;
    let mut pwm = pca9685::Pca9685::new(i2c, PCA9685_ADDRESS, PWM_FREQUENCY_HZ)
        .context("failed to initialize PCA9685 at address 0x40")?;

    loop {
        log::info!("moving channels forward to 180 degrees");
        set_all_channels(&mut pwm, 180.0)?;
        sleep(HOLD_DELAY);

        log::info!("moving channels backward to 0 degrees");
        set_all_channels(&mut pwm, 0.0)?;
        sleep(HOLD_DELAY);
    }
}
