use serde::{Deserialize, Serialize};

pub const JOINT_COUNT: usize = 6;

pub const ARM_UDP_PORT: u16 = 8888;
pub const CHASSIS_CONTROL_PORT: u16 = 4210;
pub const CHASSIS_TELEMETRY_PORT: u16 = 4211;

pub fn arm_udp_bind() -> String {
    format!("0.0.0.0:{ARM_UDP_PORT}")
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArmCommand {
    JointGoal {
        name: Vec<String>,
        position: Vec<f32>,
        velocity: Option<Vec<f32>>,
        seq: Option<u64>,
    },
    SpeedScale { value: f32 },
    Ping,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JointState {
    pub r#type: String,
    pub name: Vec<String>,
    pub position: Vec<f32>,
    pub velocity: Option<Vec<f32>>,
}

pub fn valid_joint_names(names: &[String]) -> bool {
    names.len() == JOINT_COUNT
        && names.iter().enumerate().all(|(index, name)| name == &format!("joint_{}", index + 1))
}
