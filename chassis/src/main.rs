mod setup;
mod wifi;

use core::sync::atomic::{AtomicBool, Ordering};
use embassy_executor::Executor;
use embassy_time::{Duration, Instant, Timer};
use esp_idf_hal::{
    gpio::PinDriver,
    ledc::{config as ledc_config, LedcDriver, LedcTimerDriver},
    peripherals::Peripherals,
    units::FromValueType as _,
};
use static_cell::StaticCell;
use std::{
    net::{Ipv4Addr, UdpSocket},
    rc::Rc,
};

const COMPUTER_CONTROL_PORT: u16 = 4210;
const TELEMETRY_PORT: u16 = 4211;
const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
const COMPUTER_PACKET_LEN: usize = 11;
const COMPUTER_FAILSAFE_MS: u64 = 300;
const CONTROL_PERIOD_MS: u64 = 20;
const MAX_DUTY_PER_MILLE: i32 = 850;
const SLEW_PER_TICK: i32 = 45;
const BOOT_CYCLE_TEST: bool = true;
const BOOT_TEST_DUTY_PER_MILLE: i32 = 300;
const BOOT_TEST_PHASE_MS: u64 = 5000;

// Change either value to -1 if that track runs backward during a forward command.
const LEFT_POLARITY: i32 = 1;
const RIGHT_POLARITY: i32 = 1;

static WIFI_READY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
struct Command {
    sequence: u16,
    left: i16,
    right: i16,
    armed: bool,
}

fn parse_packet(data: &[u8]) -> Option<Command> {
    if data.len() != COMPUTER_PACKET_LEN || &data[0..4] != b"AUT1" {
        return None;
    }
    Some(Command {
        sequence: u16::from_le_bytes([data[4], data[5]]),
        left: i16::from_le_bytes([data[6], data[7]]).clamp(-1000, 1000),
        right: i16::from_le_bytes([data[8], data[9]]).clamp(-1000, 1000),
        armed: data[10] & 1 != 0,
    })
}

fn sequence_is_new(new: u16, old: u16) -> bool {
    let delta = new.wrapping_sub(old);
    delta != 0 && delta < 0x8000
}

fn slew(current: i32, target: i32) -> i32 {
    current + (target - current).clamp(-SLEW_PER_TICK, SLEW_PER_TICK)
}

fn set_signed(
    forward: &mut LedcDriver<'_>,
    reverse: &mut LedcDriver<'_>,
    command: i32,
    max_duty: u32,
) {
    let duty = command.unsigned_abs().min(max_duty);
    if command > 0 {
        reverse.set_duty(0).unwrap();
        forward.set_duty(duty).unwrap();
    } else if command < 0 {
        forward.set_duty(0).unwrap();
        reverse.set_duty(duty).unwrap();
    } else {
        forward.set_duty(0).unwrap();
        reverse.set_duty(0).unwrap();
    }
}

#[embassy_executor::task]
async fn wifi_task(modem: esp_idf_svc::hal::modem::Modem) {
    match wifi::connect(modem).await {
        Ok(_wifi) => {
            WIFI_READY.store(true, Ordering::Release);
            loop {
                Timer::after(Duration::from_secs(3600)).await;
            }
        }
        Err(err) => {
            log::error!("Wi-Fi failed: {err:?}");
            WIFI_READY.store(false, Ordering::Release);
            loop {
                Timer::after(Duration::from_secs(3600)).await;
            }
        }
    }
}

#[embassy_executor::task]
async fn drive_task(
    pin25: esp_idf_hal::gpio::Gpio25,
    pin26: esp_idf_hal::gpio::Gpio26,
    pin32: esp_idf_hal::gpio::Gpio32,
    pin33: esp_idf_hal::gpio::Gpio33,
    pin27: esp_idf_hal::gpio::Gpio27,
    pin14: esp_idf_hal::gpio::Gpio14,
    ledc: esp_idf_hal::ledc::LEDC,
) {
    let mut left_en = PinDriver::output(pin27).unwrap();
    let mut right_en = PinDriver::output(pin14).unwrap();
    left_en.set_low().unwrap();
    right_en.set_low().unwrap();

    let timer_config = ledc_config::TimerConfig::new()
        .frequency(20u32.kHz().into())
        .resolution(ledc_config::Resolution::Bits10);
    let timer = Rc::new(LedcTimerDriver::new(ledc.timer0, &timer_config).unwrap());

    let mut left_forward = LedcDriver::new(ledc.channel0, timer.clone(), pin25).unwrap();
    let mut left_reverse = LedcDriver::new(ledc.channel1, timer.clone(), pin26).unwrap();
    let mut right_forward = LedcDriver::new(ledc.channel2, timer.clone(), pin32).unwrap();
    let mut right_reverse = LedcDriver::new(ledc.channel3, timer, pin33).unwrap();
    let hw_max = left_forward.get_max_duty();
    let limit = hw_max as i32 * MAX_DUTY_PER_MILLE / 1000;

    set_signed(&mut left_forward, &mut left_reverse, 0, hw_max);
    set_signed(&mut right_forward, &mut right_reverse, 0, hw_max);

    let mut buf = [0u8; 64];
    let mut socket: Option<UdpSocket> = None;
    let mut computer_sequence: Option<u16> = None;
    let mut computer_last_valid = Instant::now();
    let mut computer_left = 0i32;
    let mut computer_right = 0i32;
    let mut computer_armed = false;
    let mut boot_test_active = BOOT_CYCLE_TEST;
    let boot_test_started_at = Instant::now();
    let mut telemetry_at = Instant::now();
    let mut telemetry_sequence = 0u32;
    let mut actual_left = 0i32;
    let mut actual_right = 0i32;

    loop {
        if socket.is_none() && WIFI_READY.load(Ordering::Acquire) {
            match UdpSocket::bind(("0.0.0.0", COMPUTER_CONTROL_PORT)) {
                Ok(s) => {
                    s.set_nonblocking(true).unwrap();
                    match s.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED) {
                        Ok(()) => {
                            s.set_multicast_ttl_v4(1).unwrap();
                            log::info!(
                                "Computer UDP multicast ready on {MULTICAST_GROUP}:{COMPUTER_CONTROL_PORT}"
                            );
                            socket = Some(s);
                        }
                        Err(err) => log::error!("Computer UDP multicast join failed: {err}"),
                    }
                }
                Err(err) => log::error!("Computer UDP bind failed: {err}"),
            }
        }

        if let Some(s) = &socket {
            while let Ok((len, _peer)) = s.recv_from(&mut buf) {
                if let Some(cmd) = parse_packet(&buf[..len]) {
                    let session_expired = Instant::now().duration_since(computer_last_valid)
                        > Duration::from_millis(COMPUTER_FAILSAFE_MS);
                    let fresh = session_expired
                        || computer_sequence
                            .map(|old| sequence_is_new(cmd.sequence, old))
                            .unwrap_or(true);
                    if fresh {
                        computer_sequence = Some(cmd.sequence);
                        computer_last_valid = Instant::now();
                        computer_left = cmd.left as i32;
                        computer_right = cmd.right as i32;
                        computer_armed = cmd.armed;
                        boot_test_active = false;
                    }
                }
            }
        }

        let computer_fresh = computer_armed
            && Instant::now().duration_since(computer_last_valid)
                <= Duration::from_millis(COMPUTER_FAILSAFE_MS);
        // Run the startup forward test once, then remain stopped until a
        // valid computer command is received.
        if boot_test_active
            && Instant::now().duration_since(boot_test_started_at)
                >= Duration::from_millis(BOOT_TEST_PHASE_MS)
        {
            boot_test_active = false;
        }

        let (target_left, target_right, source) = if boot_test_active {
            (
                limit * BOOT_TEST_DUTY_PER_MILLE / 1000,
                limit * BOOT_TEST_DUTY_PER_MILLE / 1000,
                "boot-test",
            )
        } else if computer_fresh {
            (
                computer_left * limit / 1000 * LEFT_POLARITY,
                computer_right * limit / 1000 * RIGHT_POLARITY,
                "computer",
            )
        } else {
            (0, 0, "stop")
        };

        if let Some(s) = &socket {
            if Instant::now() >= telemetry_at {
                let payload = format!(
                    "{{\"v\":1,\"seq\":{},\"source\":\"{}\",\"auto\":true,\"target_left\":{},\"target_right\":{},\"pwm_left\":{},\"pwm_right\":{}}}",
                    telemetry_sequence, source, target_left, target_right, actual_left, actual_right
                );
                let _ = s.send_to(payload.as_bytes(), (MULTICAST_GROUP, TELEMETRY_PORT));
                telemetry_sequence = telemetry_sequence.wrapping_add(1);
                telemetry_at = Instant::now() + Duration::from_millis(100);
            }
        }

        if target_left == 0 && target_right == 0 {
            actual_left = 0;
            actual_right = 0;
            set_signed(&mut left_forward, &mut left_reverse, 0, hw_max);
            set_signed(&mut right_forward, &mut right_reverse, 0, hw_max);
            left_en.set_low().unwrap();
            right_en.set_low().unwrap();
        } else {
            left_en.set_high().unwrap();
            right_en.set_high().unwrap();
            actual_left = slew(actual_left, target_left);
            actual_right = slew(actual_right, target_right);
            set_signed(&mut left_forward, &mut left_reverse, actual_left, hw_max);
            set_signed(&mut right_forward, &mut right_reverse, actual_right, hw_max);
        }
        Timer::after(Duration::from_millis(CONTROL_PERIOD_MS)).await;
    }
}

fn main() {
    setup::init();
    let peripherals = Peripherals::take().unwrap();
    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| {
        spawner.spawn(wifi_task(peripherals.modem)).unwrap();
        spawner
            .spawn(drive_task(
                peripherals.pins.gpio25,
                peripherals.pins.gpio26,
                peripherals.pins.gpio32,
                peripherals.pins.gpio33,
                peripherals.pins.gpio27,
                peripherals.pins.gpio14,
                peripherals.ledc,
            ))
            .unwrap();
    });
}
