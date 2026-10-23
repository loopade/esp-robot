use embassy_time::Timer;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::arm::{SharedArmState, ARM_CONFIG, JOINT_COUNT};
use crate::pca9685;
use crate::protocol::JointStatePacket;
use anyhow::{Context, Result};
use esp_idf_hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_hal::units::FromValueType as _;
use esp_idf_svc::nvs::EspNvs;

const UDP_BIND_ADDR: &str = "0.0.0.0:8888";
const CONTROL_LOOP_PERIOD: Duration = Duration::from_millis(33);
const TELEMETRY_INTERVAL: Duration = Duration::from_millis(100);
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(1);
const TELEMETRY_MULTICAST: &str = "239.255.42.99:4211";
const PCA9685_ADDRESS: u8 = 0x40;

pub async fn arm_run(
    nvs_partition: esp_idf_svc::nvs::EspDefaultNvsPartition,
    i2c0: esp_idf_hal::i2c::I2C0,
    gpio21: esp_idf_hal::gpio::Gpio21,
    gpio22: esp_idf_hal::gpio::Gpio22,
) -> Result<()> {
    log::info!("starting esp32-arm firmware");

    let i2c_config = I2cConfig::new().baudrate(100.kHz().into());
    let i2c = I2cDriver::new(i2c0, gpio21, gpio22, &i2c_config)
        .context("failed to initialize I2C0 on GPIO21/GPIO22")?;
    let pwm = pca9685::Pca9685::new(i2c, PCA9685_ADDRESS, 50.0)
        .context("failed to initialize PCA9685")?;

    let nvs = EspNvs::new(nvs_partition, "arm_pose", true)
        .context("failed to open arm pose NVS namespace")?;
    let mut startup_pose = [0.0_f32; JOINT_COUNT];
    let mut stored_pose = [0_u8; JOINT_COUNT * 4];
    let calibrated = match nvs.get_blob("startup", &mut stored_pose) {
        Ok(Some(blob)) if blob.len() == JOINT_COUNT * 4 => {
            for (index, chunk) in blob.chunks_exact(4).enumerate() {
                startup_pose[index] = f32::from_le_bytes(chunk.try_into().unwrap());
            }
            startup_pose.iter().all(|value| value.is_finite())
        }
        _ => false,
    };
    if calibrated {
        log::info!("loaded saved startup arm pose: {:?}", startup_pose);
    } else {
        startup_pose = ARM_CONFIG.home_position_deg;
        log::info!("no saved startup arm pose; using configured home pose");
    }

    let mut minimum = ARM_CONFIG.min_angle_deg;
    let mut maximum = ARM_CONFIG.max_angle_deg;
    let mut stored_limits = [0_u8; JOINT_COUNT * 8];
    if let Ok(Some(blob)) = nvs.get_blob("joint_limits", &mut stored_limits) {
        if blob.len() == JOINT_COUNT * 8 {
            for i in 0..JOINT_COUNT {
                minimum[i] = f32::from_le_bytes(blob[i * 4..i * 4 + 4].try_into().unwrap());
                let offset = JOINT_COUNT * 4 + i * 4;
                maximum[i] = f32::from_le_bytes(blob[offset..offset + 4].try_into().unwrap());
            }
        }
    }
    let mut state = SharedArmState::new(&ARM_CONFIG, startup_pose, calibrated);
    let _ = state.set_joint_limits(minimum, maximum);
    let socket = UdpSocket::bind(UDP_BIND_ADDR)
        .with_context(|| format!("failed to bind {UDP_BIND_ADDR}"))?;
    socket
        .set_nonblocking(true)
        .context("failed to switch UDP socket to nonblocking mode")?;
    log::info!("listening for arm commands on udp://{UDP_BIND_ADDR}");

    control_loop(pwm, socket, state, nvs).await;
}

fn receive_pending_commands(
    socket: &UdpSocket,
    state: &mut SharedArmState,
    nvs: &mut EspNvs<esp_idf_svc::nvs::NvsDefault>,
) {
    let mut buffer = [0_u8; 1024];

    loop {
        match socket.recv_from(&mut buffer) {
            Ok((size, peer)) => handle_command(&buffer[..size], peer, state, nvs),
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) => {
                log::error!("udp receive failed: {error}");
                break;
            }
        }
    }
}

fn handle_command(
    packet: &[u8],
    peer: SocketAddr,
    state: &mut SharedArmState,
    nvs: &mut EspNvs<esp_idf_svc::nvs::NvsDefault>,
) {
    let text = match core::str::from_utf8(packet) {
        Ok(value) => value,
        Err(_) => return,
    };
    if text.contains("\"type\":\"set_startup_pose\"") {
        if let Some(radians) = extract_positions(text) {
            let values = radians.map(f32::to_degrees);
            let mut bytes = [0_u8; JOINT_COUNT * 4];
            for (index, value) in values.iter().enumerate() {
                bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            if let Err(error) = nvs.set_blob("startup", &bytes) {
                log::error!("failed to save startup pose: {error}");
                return;
            }
            state.set_startup_pose(values);
            log::info!("saved startup arm pose: {:?}", values);
        }
        return;
    }
    if text.contains("\"type\":\"set_joint_limits\"") {
        let Some(minimum) = extract_array(text, "min_deg") else {
            return;
        };
        let Some(maximum) = extract_array(text, "max_deg") else {
            return;
        };
        if minimum
            .iter()
            .zip(maximum.iter())
            .any(|(min, max)| min >= max)
        {
            return;
        }
        let mut bytes = [0_u8; JOINT_COUNT * 8];
        for i in 0..JOINT_COUNT {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&minimum[i].to_le_bytes());
            let offset = JOINT_COUNT * 4 + i * 4;
            bytes[offset..offset + 4].copy_from_slice(&maximum[i].to_le_bytes());
        }
        if let Err(error) = nvs.set_blob("joint_limits", &bytes) {
            log::error!("failed to save joint limits: {error}");
            return;
        }
        if state.set_joint_limits(minimum, maximum) {
            log::info!("saved joint limits min={minimum:?} max={maximum:?}");
        }
        return;
    }
    if !text.contains("\"type\":\"joint_goal\"") {
        return;
    }
    let Some(start) = text.find("\"position\":[") else {
        return;
    };
    // `"position":[` is 12 bytes; keep the first value (including its sign).
    let rest = &text[start + 12..];
    let Some(end) = rest.find(']') else {
        return;
    };
    let values: Vec<f32> = rest[..end]
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    if values.len() != JOINT_COUNT {
        return;
    }
    let mut positions_deg = [0.0; JOINT_COUNT];
    for (index, value) in values.into_iter().enumerate() {
        positions_deg[index] = value.to_degrees();
    }
    let speed_scale = extract_number(text, "speed_scale");

    state.set_last_peer(peer);
    state.apply_goal(positions_deg, speed_scale, None);
}

fn extract_positions(text: &str) -> Option<[f32; JOINT_COUNT]> {
    let start = text.find("\"position\":[")? + 12;
    let rest = &text[start..];
    let end = rest.find(']')?;
    let values: Vec<f32> = rest[..end]
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    if values.len() != JOINT_COUNT || values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let mut result = [0.0; JOINT_COUNT];
    result.copy_from_slice(&values);
    Some(result)
}

fn extract_array(text: &str, key: &str) -> Option<[f32; JOINT_COUNT]> {
    let marker = format!("\"{key}\":[");
    let start = text.find(&marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find(']')?;
    let values: Vec<f32> = rest[..end]
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    if values.len() != JOINT_COUNT || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let mut result = [0.0; JOINT_COUNT];
    result.copy_from_slice(&values);
    Some(result)
}

fn extract_number(text: &str, key: &str) -> Option<f32> {
    let marker = format!("\"{key}\":");
    let start = text.find(&marker)? + marker.len();
    let value = text[start..]
        .split(|character| character == ',' || character == '}' || character == ']')
        .next()?
        .trim();
    value.parse().ok()
}

async fn control_loop(
    mut pwm: pca9685::Pca9685<'_>,
    socket: UdpSocket,
    mut state: SharedArmState,
    mut nvs: EspNvs<esp_idf_svc::nvs::NvsDefault>,
) -> ! {
    if let Err(error) = apply_joint_positions(&mut pwm, &state.current_deg) {
        log::error!("failed to apply initial pose: {error:?}");
    }

    let mut last_telemetry = Instant::now();
    let mut last_discovery = Instant::now();
    let telemetry_multicast: SocketAddr = TELEMETRY_MULTICAST
        .parse()
        .expect("valid telemetry multicast address");

    loop {
        let cycle_started = Instant::now();
        receive_pending_commands(&socket, &mut state, &mut nvs);

        let mut positions_to_apply = None;
        let mut telemetry_packet = None;
        let mut telemetry_peer = None;

        if state.step() {
            positions_to_apply = Some(state.current_deg);
        }

        if last_telemetry.elapsed() >= TELEMETRY_INTERVAL {
            telemetry_packet = Some(JointStatePacket::from(state.telemetry()));
            telemetry_peer = state.last_peer;
        }
        if last_discovery.elapsed() >= DISCOVERY_INTERVAL {
            let hello = b"{\"type\":\"hello\",\"device\":\"arm\",\"name\":\"arm-esp32\",\"control_port\":8888}";
            let _ = socket.send_to(hello, "239.255.42.99:4209");
            // Match chassis discovery: advertise on the telemetry multicast stream.
            let _ = socket.send_to(hello, "239.255.42.99:4211");
            last_discovery = Instant::now();
        }

        if let Some(positions) = positions_to_apply {
            if let Err(error) = apply_joint_positions(&mut pwm, &positions) {
                log::error!("failed to apply arm pose: {error:?}");
            }
        }

        if let Some(packet) = telemetry_packet {
            // The control center listens on the telemetry multicast port. Keep
            // publishing even before the first command so a refreshed page can
            // immediately display the actual ESP32 pose.
            if let Err(error) = send_telemetry(&socket, telemetry_multicast, &packet) {
                log::warn!("failed to multicast arm telemetry: {error}");
            }
            if let Some(peer) = telemetry_peer {
                if let Err(error) = send_telemetry(&socket, peer, &packet) {
                    log::warn!("failed to send telemetry to {peer}: {error}");
                }
            }
            last_telemetry = Instant::now();
        }

        let elapsed = cycle_started.elapsed();
        if elapsed < CONTROL_LOOP_PERIOD {
            Timer::after(embassy_time::Duration::from_millis(
                (CONTROL_LOOP_PERIOD - elapsed).as_millis() as u64,
            ))
            .await;
        }
    }
}

fn apply_joint_positions(
    pwm: &mut pca9685::Pca9685<'_>,
    joint_positions_deg: &[f32; JOINT_COUNT],
) -> Result<()> {
    for (index, channel) in ARM_CONFIG.servo_channels.iter().enumerate() {
        let servo_angle = ARM_CONFIG.joint_to_servo_angle(index, joint_positions_deg[index]);
        pwm.set_servo_angle(
            *channel,
            servo_angle,
            ARM_CONFIG.servo_min_deg[index],
            ARM_CONFIG.servo_max_deg[index],
            ARM_CONFIG.servo_pulse_min_us[index],
            ARM_CONFIG.servo_pulse_max_us[index],
        )?;
    }

    for (channel, angle) in ARM_CONFIG
        .fixed_output_channels
        .iter()
        .zip(ARM_CONFIG.fixed_output_angles_deg.iter())
    {
        pwm.set_servo_angle(*channel, *angle, 0.0, 180.0, 500.0, 2500.0)?;
    }

    Ok(())
}

fn send_telemetry(socket: &UdpSocket, peer: SocketAddr, packet: &JointStatePacket) -> Result<()> {
    let payload = serde_json::to_vec(packet)?;
    socket.send_to(&payload, peer)?;
    Ok(())
}
