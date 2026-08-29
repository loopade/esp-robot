use anyhow::Result;
use esp_idf_hal::delay::{FreeRtos, BLOCK};
use esp_idf_hal::i2c::I2cDriver;

const MODE1: u8 = 0x00;
const MODE2: u8 = 0x01;
const PRESCALE: u8 = 0xfe;
const LED0_ON_L: u8 = 0x06;
const ALL_LED_ON_L: u8 = 0xfa;
const WRITE_RETRY_COUNT: usize = 3;
const WRITE_RETRY_DELAY_MS: u32 = 2;

pub struct Pca9685<'d> {
    i2c: I2cDriver<'d>,
    address: u8,
    pwm_frequency_hz: f32,
}

impl<'d> Pca9685<'d> {
    pub fn new(i2c: I2cDriver<'d>, address: u8, pwm_frequency_hz: f32) -> Result<Self> {
        let mut controller = Self {
            i2c,
            address,
            pwm_frequency_hz,
        };
        controller.initialize()?;
        Ok(controller)
    }

    pub fn set_all_off(&mut self) -> Result<()> {
        self.write_bytes_with_retry(
            &[ALL_LED_ON_L, 0x00, 0x00, 0x00, 0x00],
            "set_all_off",
        )?;
        Ok(())
    }

    pub fn set_servo_angle(
        &mut self,
        channel: u8,
        angle_deg: f32,
        servo_min_deg: f32,
        servo_max_deg: f32,
        pulse_min_us: f32,
        pulse_max_us: f32,
    ) -> Result<()> {
        let clamped_angle = angle_deg.clamp(servo_min_deg, servo_max_deg);
        let span_deg = (servo_max_deg - servo_min_deg).max(1e-6);
        let ratio = (clamped_angle - servo_min_deg) / span_deg;
        let pulse_us = pulse_min_us + ratio * (pulse_max_us - pulse_min_us);
        let off_count = self.pulse_us_to_counts(pulse_us);
        self.set_channel_counts(channel, 0, off_count)
    }

    fn initialize(&mut self) -> Result<()> {
        self.write_reg(MODE1, 0x10)?;
        self.write_reg(PRESCALE, self.compute_prescale())?;
        self.write_reg(MODE2, 0x04)?;
        self.write_reg(MODE1, 0x20)?;
        FreeRtos::delay_ms(5);
        self.write_reg(MODE1, 0xa0)?;
        FreeRtos::delay_ms(1);
        self.set_all_off()?;
        Ok(())
    }

    fn compute_prescale(&self) -> u8 {
        let value = (25_000_000.0 / (4096.0 * self.pwm_frequency_hz)).round() - 1.0;
        value.clamp(3.0, 255.0) as u8
    }

    fn pulse_us_to_counts(&self, pulse_us: f32) -> u16 {
        let period_us = 1_000_000.0 / self.pwm_frequency_hz;
        let counts = ((pulse_us / period_us) * 4096.0).round();
        counts.clamp(0.0, 4095.0) as u16
    }

    fn set_channel_counts(&mut self, channel: u8, on_count: u16, off_count: u16) -> Result<()> {
        let register = LED0_ON_L + channel.saturating_mul(4);
        let data = [
            register,
            (on_count & 0xff) as u8,
            (on_count >> 8) as u8,
            (off_count & 0xff) as u8,
            (off_count >> 8) as u8,
        ];
        self.write_bytes_with_retry(&data, &format!("set_channel_counts(channel={channel})"))?;
        Ok(())
    }

    fn write_reg(&mut self, register: u8, value: u8) -> Result<()> {
        self.write_bytes_with_retry(&[register, value], &format!("write_reg(0x{register:02x})"))?;
        Ok(())
    }

    fn write_bytes_with_retry(&mut self, data: &[u8], context: &str) -> Result<()> {
        let mut last_error = None;

        for attempt in 1..=WRITE_RETRY_COUNT {
            match self.i2c.write(self.address, data, BLOCK) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    log::warn!(
                        "PCA9685 I2C write failed during {context}, attempt {attempt}/{WRITE_RETRY_COUNT}: {error:?}"
                    );
                    last_error = Some(error);
                    if attempt < WRITE_RETRY_COUNT {
                        FreeRtos::delay_ms(WRITE_RETRY_DELAY_MS);
                    }
                }
            }
        }

        Err(last_error.expect("retry loop must capture at least one error").into())
    }
}
