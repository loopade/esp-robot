use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    env,
    io::{self, Read},
    net::{Ipv4Addr, UdpSocket},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tiny_http::{Header, Method, Response, Server, StatusCode};

const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
const CONTROL_DESTINATION: &str = "239.255.42.99:4210";
const TELEMETRY_BIND: &str = "0.0.0.0:4211";
const WEB_BIND: &str = "127.0.0.1:8080";
const DISCOVERY_BIND: &str = "0.0.0.0:4209";
const PERIOD: Duration = Duration::from_millis(20);
const DEADMAN: Duration = Duration::from_millis(350);
const INDEX_HTML: &str = include_str!("../web/index.html");

#[derive(Clone, Copy)]
struct ControlState {
    left: i16,
    right: i16,
    armed: bool,
    updated: Instant,
}

impl Default for ControlState {
    fn default() -> Self {
        Self {
            left: 0,
            right: 0,
            armed: false,
            updated: Instant::now() - DEADMAN,
        }
    }
}

#[derive(Default)]
struct TelemetryState {
    peer: Option<String>,
    payload: Option<Value>,
    updated: Option<Instant>,
    devices: Vec<Value>,
}

#[derive(Deserialize)]
struct ControlRequest {
    left: i16,
    right: i16,
    armed: bool,
}

fn packet(sequence: u16, left: i16, right: i16, armed: bool) -> [u8; 11] {
    let mut data = [0u8; 11];
    data[0..4].copy_from_slice(b"AUT1");
    data[4..6].copy_from_slice(&sequence.to_le_bytes());
    data[6..8].copy_from_slice(&left.clamp(-1000, 1000).to_le_bytes());
    data[8..10].copy_from_slice(&right.clamp(-1000, 1000).to_le_bytes());
    data[10] = armed as u8;
    data
}

fn sequence_seed() -> u16 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u16
}

fn monitor() -> io::Result<()> {
    let socket = UdpSocket::bind(TELEMETRY_BIND)?;
    socket.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)?;
    let mut buf = [0u8; 2048];
    println!("Telemetry on multicast {MULTICAST_GROUP}:4211 (Ctrl+C to stop)");
    loop {
        let (len, peer) = socket.recv_from(&mut buf)?;
        println!("{peer} {}", String::from_utf8_lossy(&buf[..len]));
    }
}

fn drive(left: i16, right: i16, seconds: f32) -> io::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_multicast_ttl_v4(1)?;
    let mut sequence = sequence_seed();
    let until = Instant::now() + Duration::from_secs_f32(seconds.max(0.0));
    while Instant::now() < until {
        socket.send_to(&packet(sequence, left, right, true), CONTROL_DESTINATION)?;
        sequence = sequence.wrapping_add(1);
        thread::sleep(PERIOD);
    }
    for _ in 0..10 {
        socket.send_to(&packet(sequence, 0, 0, false), CONTROL_DESTINATION)?;
        sequence = sequence.wrapping_add(1);
        thread::sleep(PERIOD);
    }
    Ok(())
}

fn arm_goal(host: &str, positions_deg: &[f32]) -> io::Result<()> {
    if positions_deg.len() != 5 { return Err(io::Error::new(io::ErrorKind::InvalidInput, "arm requires 5 joint angles")); }
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    let names: Vec<String> = (1..=5).map(|i| format!("joint_{i}")).collect();
    let position: Vec<f32> = positions_deg.iter().map(|value| value.to_radians()).collect();
    let payload = json!({"type": "joint_goal", "name": names, "position": position});
    socket.send_to(payload.to_string().as_bytes(), format!("{host}:8888"))?;
    println!("sent JointState-compatible arm goal to {host}:8888");
    Ok(())
}

fn command_loop(control: Arc<Mutex<ControlState>>) -> io::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_multicast_ttl_v4(1)?;
    let mut sequence = sequence_seed();
    loop {
        let state = *control.lock().unwrap();
        let active = state.armed && state.updated.elapsed() <= DEADMAN;
        let values = if active {
            (state.left, state.right, true)
        } else {
            (0, 0, false)
        };
        socket.send_to(
            &packet(sequence, values.0, values.1, values.2),
            CONTROL_DESTINATION,
        )?;
        sequence = sequence.wrapping_add(1);
        thread::sleep(PERIOD);
    }
}

fn telemetry_loop(telemetry: Arc<Mutex<TelemetryState>>) -> io::Result<()> {
    let socket = UdpSocket::bind(TELEMETRY_BIND)?;
    socket.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)?;
    let mut buf = [0u8; 2048];
    loop {
        let (len, peer) = socket.recv_from(&mut buf)?;
        if let Ok(payload) = serde_json::from_slice::<Value>(&buf[..len]) {
            let mut state = telemetry.lock().unwrap();
            let packet_type = payload.get("type").and_then(Value::as_str);
            if payload.get("device").and_then(Value::as_str) == Some("arm") {
                let mut device = payload.clone();
                if let Some(object) = device.as_object_mut() { object.insert("peer".into(), json!(peer.ip().to_string())); }
                state.devices.retain(|item| item.get("peer") != Some(&json!(peer.ip().to_string())));
                state.devices.push(device);
            }
            // Discovery hello packets share this multicast port with arm
            // telemetry. Keep the last actual telemetry packet available to
            // the web page instead of replacing it with a packet without pose.
            if packet_type != Some("hello") {
                state.peer = Some(peer.to_string());
                state.payload = Some(payload);
                state.updated = Some(Instant::now());
            }
        }
    }
}

fn discovery_loop(telemetry: Arc<Mutex<TelemetryState>>) -> io::Result<()> {
    let socket = UdpSocket::bind(DISCOVERY_BIND)?;
    socket.set_broadcast(true)?;
    socket.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)?;
    let mut buf = [0u8; 2048];
    loop {
        let (len, peer) = socket.recv_from(&mut buf)?;
        if let Ok(mut payload) = serde_json::from_slice::<Value>(&buf[..len]) {
            if let Some(object) = payload.as_object_mut() { object.insert("peer".into(), json!(peer.ip().to_string())); }
            let mut state = telemetry.lock().unwrap();
            state.devices.retain(|item| item.get("peer") != Some(&json!(peer.ip().to_string())));
            state.devices.push(payload);
        }
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).unwrap()
}

fn json_response(value: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(value.to_string())
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
}

fn web() -> io::Result<()> {
    let control = Arc::new(Mutex::new(ControlState::default()));
    let telemetry = Arc::new(Mutex::new(TelemetryState::default()));
    {
        let state = Arc::clone(&control);
        thread::spawn(move || {
            if let Err(error) = command_loop(state) {
                eprintln!("Command UDP error: {error}")
            }
        });
    }
    {
        let state = Arc::clone(&telemetry);
        thread::spawn(move || { let _ = discovery_loop(state); });
    }
    {
        let state = Arc::clone(&telemetry);
        thread::spawn(move || {
            if let Err(error) = telemetry_loop(state) {
                eprintln!("Telemetry UDP error: {error}")
            }
        });
    }

    let server = Server::http(WEB_BIND).map_err(|error| io::Error::other(error.to_string()))?;
    println!("Tracked chassis control: http://{WEB_BIND}");
    println!("Keep this process running; Ctrl+C stops the controller.");
    for mut request in server.incoming_requests() {
        let path = request
            .url()
            .split('?')
            .next()
            .unwrap_or(request.url())
            .to_owned();
        match (request.method(), path.as_str()) {
            (&Method::Get, "/") => {
                let response = Response::from_string(INDEX_HTML)
                    .with_header(header("Content-Type", "text/html; charset=utf-8"))
                    .with_header(header("Cache-Control", "no-store"));
                let _ = request.respond(response);
            }
            (&Method::Get, "/api/state") => {
                let command = *control.lock().unwrap();
                let telemetry = telemetry.lock().unwrap();
                let age = telemetry
                    .updated
                    .map(|time| time.elapsed().as_millis() as u64);
                let _ = request.respond(json_response(json!({
                    "command": {"left":command.left,"right":command.right,"armed":command.armed && command.updated.elapsed() <= DEADMAN},
                    "telemetry":telemetry.payload,"peer":telemetry.peer,"telemetry_age_ms":age,
                    "online": age.map(|value| value < 1000).unwrap_or(false) || !telemetry.devices.is_empty(),
                    "devices": telemetry.devices
                })));
            }
            (&Method::Post, "/api/control") => {
                let mut body = String::new();
                let parsed = request
                    .as_reader()
                    .take(4096)
                    .read_to_string(&mut body)
                    .ok()
                    .and_then(|_| serde_json::from_str::<ControlRequest>(&body).ok());
                if let Some(input) = parsed {
                    let mut state = control.lock().unwrap();
                    state.left = input.left.clamp(-1000, 1000);
                    state.right = input.right.clamp(-1000, 1000);
                    state.armed = input.armed;
                    state.updated = Instant::now();
                    let _ = request.respond(json_response(json!({"ok":true})));
                } else {
                    let _ = request.respond(
                        json_response(json!({"error":"invalid request"}))
                            .with_status_code(StatusCode(400)),
                    );
                }
            }
            (&Method::Post, "/api/stop") => {
                let mut state = control.lock().unwrap();
                state.left = 0;
                state.right = 0;
                state.armed = false;
                state.updated = Instant::now();
                let _ = request.respond(json_response(json!({"ok":true})));
            }
            (&Method::Post, "/api/arm/startup") => {
                let mut body = String::new();
                let parsed = request.as_reader().take(4096).read_to_string(&mut body).ok().and_then(|_| serde_json::from_str::<Value>(&body).ok());
                if let Some(value) = parsed { if let (Some(host), Some(position)) = (value.get("host").and_then(Value::as_str), value.get("position").and_then(Value::as_array)) {
                    let valid = position.len() == 5 && position.iter().all(|v| v.as_f64().map(|n| n.is_finite()).unwrap_or(false));
                    if !valid { let _ = request.respond(json_response(json!({"ok":false,"error":"position must contain five finite numbers"})).with_status_code(StatusCode(400))); continue; }
                    let payload = json!({"type":"set_startup_pose","position":position});
                    match UdpSocket::bind("0.0.0.0:0").and_then(|socket| socket.send_to(payload.to_string().as_bytes(), format!("{host}:8888"))) {
                        Ok(_) => { let _ = request.respond(json_response(json!({"ok":true}))); }
                        Err(error) => { let _ = request.respond(json_response(json!({"ok":false,"error":error.to_string()})).with_status_code(StatusCode(502))); }
                    }
                } else { let _ = request.respond(json_response(json!({"ok":false})).with_status_code(StatusCode(400))); } } else { let _ = request.respond(json_response(json!({"ok":false})).with_status_code(StatusCode(400))); }
            }
            (&Method::Post, "/api/arm/limits") => {
                let mut body = String::new();
                let parsed = request.as_reader().take(4096).read_to_string(&mut body).ok().and_then(|_| serde_json::from_str::<Value>(&body).ok());
                if let Some(value) = parsed { if let (Some(host), Some(minimum), Some(maximum)) = (value.get("host").and_then(Value::as_str), value.get("min_deg").and_then(Value::as_array), value.get("max_deg").and_then(Value::as_array)) {
                    let valid = minimum.len() == 5 && maximum.len() == 5 && minimum.iter().zip(maximum.iter()).all(|(a,b)| a.as_f64().zip(b.as_f64()).map(|(a,b)| a.is_finite() && b.is_finite() && a < b).unwrap_or(false));
                    if !valid { let _ = request.respond(json_response(json!({"ok":false,"error":"five valid min/max pairs required"})).with_status_code(StatusCode(400))); continue; }
                    let payload = json!({"type":"set_joint_limits","min_deg":minimum,"max_deg":maximum});
                    match UdpSocket::bind("0.0.0.0:0").and_then(|socket| socket.send_to(payload.to_string().as_bytes(), format!("{host}:8888"))) {
                        Ok(_) => { let _ = request.respond(json_response(json!({"ok":true}))); }
                        Err(error) => { let _ = request.respond(json_response(json!({"ok":false,"error":error.to_string()})).with_status_code(StatusCode(502))); }
                    }
                } else { let _ = request.respond(json_response(json!({"ok":false})).with_status_code(StatusCode(400))); } } else { let _ = request.respond(json_response(json!({"ok":false})).with_status_code(StatusCode(400))); }
            }
            (&Method::Post, "/api/arm") => {
                let mut body = String::new();
                let parsed = request.as_reader().take(4096).read_to_string(&mut body).ok().and_then(|_| serde_json::from_str::<Value>(&body).ok());
                if let Some(value) = parsed { if let (Some(host), Some(position)) = (value.get("host").and_then(Value::as_str), value.get("position").and_then(Value::as_array)) { let socket = UdpSocket::bind("0.0.0.0:0"); if let Ok(socket) = socket { let speed_scale = value.get("speed_scale").and_then(Value::as_f64).unwrap_or(1.0); let payload = json!({"type":"joint_goal","name":["joint_1","joint_2","joint_3","joint_4","joint_5"],"position":position,"speed_scale":speed_scale}); let _ = socket.send_to(payload.to_string().as_bytes(), format!("{host}:8888")); } let _ = request.respond(json_response(json!({"ok":true}))); } else { let _ = request.respond(json_response(json!({"ok":false})).with_status_code(StatusCode(400))); } } else { let _ = request.respond(json_response(json!({"ok":false})).with_status_code(StatusCode(400))); }
            }
            _ => {
                let _ = request.respond(Response::empty(StatusCode(404)));
            }
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        None | Some("web") => web(),
        Some("monitor") => monitor(),
        Some("drive") if args.len() == 5 => drive(
            args[2].parse().expect("left must be -1000..1000"),
            args[3].parse().expect("right must be -1000..1000"),
            args[4].parse().expect("seconds must be numeric"),
        ),
        Some("arm") if args.len() == 8 => arm_goal(&args[2], &args[3..8].iter().map(|v| v.parse().expect("joint angle must be numeric")).collect::<Vec<f32>>()),
        _ => {
            eprintln!("Usage:\n  robot-control-center web\n  robot-control-center monitor\n  robot-control-center drive LEFT RIGHT SECONDS\n  robot-control-center arm HOST J1 J2 J3 J4 J5");
            Ok(())
        }
    }
}
