mod arm;
mod pca9685;
mod protocol;
mod setup;
mod wifi;

use std::convert::TryInto;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use arm::{SharedArmState, ARM_CONFIG, JOINT_COUNT};
use esp_idf_hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::units::FromValueType as _;
use protocol::{ArmCommand, JointStatePacket};

const UDP_BIND_ADDR: &str = match option_env!("ARM_UDP_BIND") {
    Some(value) => value,
    None => "0.0.0.0:8888",
};
const CONTROL_LOOP_PERIOD: Duration = Duration::from_millis(33);
const TELEMETRY_INTERVAL: Duration = Duration::from_millis(100);
const PCA9685_ADDRESS: u8 = 0x40;

fn main() -> Result<()> {
    setup::init();
    log::info!("starting esp32-arm firmware");

    let peripherals = Peripherals::take().context("failed to take ESP32 peripherals")?;
    let _wifi = wifi::connect(peripherals.modem).context("wifi setup failed")?;

    let i2c_config = I2cConfig::new().baudrate(100.kHz().into());
    let i2c = I2cDriver::new(
        peripherals.i2c0,
        peripherals.pins.gpio21,
        peripherals.pins.gpio22,
        &i2c_config,
    )
    .context("failed to initialize I2C0 on GPIO21/GPIO22")?;
    let pwm = pca9685::Pca9685::new(i2c, PCA9685_ADDRESS, 50.0)
        .context("failed to initialize PCA9685")?;

    let state = SharedArmState::new(&ARM_CONFIG);
    let socket = UdpSocket::bind(UDP_BIND_ADDR)
        .with_context(|| format!("failed to bind {UDP_BIND_ADDR}"))?;
    socket
        .set_nonblocking(true)
        .context("failed to switch UDP socket to nonblocking mode")?;
    log::info!("listening for arm commands on udp://{UDP_BIND_ADDR}");

    control_loop(pwm, socket, state);

    #[allow(unreachable_code)]
    {
        Ok(())
    }
}

fn receive_pending_commands(socket: &UdpSocket, state: &mut SharedArmState) {
    let mut buffer = [0_u8; 1024];

    loop {
        match socket.recv_from(&mut buffer) {
            Ok((size, peer)) => handle_command(&buffer[..size], peer, state),
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) => {
                log::error!("udp receive failed: {error}");
                break;
            }
        }
    }
}

fn handle_command(packet: &[u8], peer: SocketAddr, state: &mut SharedArmState) {
    let command = match serde_json::from_slice::<ArmCommand>(packet) {
        Ok(command) => command,
        Err(error) => {
            log::warn!("ignoring invalid command from {peer}: {error}");
            return;
        }
    };

    state.set_last_peer(peer);

    match command {
        ArmCommand::JointGoal {
            positions_deg,
            speed_scale,
            seq,
        } => {
            let positions_deg: [f32; JOINT_COUNT] = match positions_deg.try_into() {
                Ok(positions) => positions,
                Err(values) => {
                    log::warn!(
                        "ignoring joint_goal from {peer}: expected {JOINT_COUNT} values, got {}",
                        values.len()
                    );
                    return;
                }
            };
            state.apply_goal(positions_deg, speed_scale, seq);
        }
        ArmCommand::SpeedScale { value } => {
            state.set_speed_scale(value);
        }
        ArmCommand::Ping => {}
    }
}

fn control_loop(mut pwm: pca9685::Pca9685<'_>, socket: UdpSocket, mut state: SharedArmState) -> ! {
    if let Err(error) = apply_joint_positions(&mut pwm, &state.current_deg) {
        log::error!("failed to apply initial pose: {error:?}");
    }

    let mut last_telemetry = Instant::now();

    loop {
        let cycle_started = Instant::now();
        receive_pending_commands(&socket, &mut state);

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

        if let Some(positions) = positions_to_apply {
            if let Err(error) = apply_joint_positions(&mut pwm, &positions) {
                log::error!("failed to apply arm pose: {error:?}");
            }
        }

        if let (Some(packet), Some(peer)) = (telemetry_packet, telemetry_peer) {
            if let Err(error) = send_telemetry(&socket, peer, &packet) {
                log::warn!("failed to send telemetry to {peer}: {error}");
            } else {
                last_telemetry = Instant::now();
            }
        }

        let elapsed = cycle_started.elapsed();
        if elapsed < CONTROL_LOOP_PERIOD {
            std::thread::sleep(CONTROL_LOOP_PERIOD - elapsed);
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
