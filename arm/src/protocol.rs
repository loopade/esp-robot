use serde::{Deserialize, Serialize};

use crate::arm::{TelemetryState, JOINT_COUNT};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArmCommand {
    JointGoal {
        positions_deg: Vec<f32>,
        speed_scale: Option<f32>,
        seq: Option<u64>,
    },
    SpeedScale {
        value: f32,
    },
    Ping,
}

#[derive(Debug, Serialize)]
pub struct JointStatePacket {
    pub r#type: &'static str,
    pub seq: u64,
    pub current_deg: [f32; JOINT_COUNT],
    pub target_deg: [f32; JOINT_COUNT],
    pub speed_scale: f32,
}

impl From<TelemetryState> for JointStatePacket {
    fn from(value: TelemetryState) -> Self {
        Self {
            r#type: "joint_state",
            seq: value.seq,
            current_deg: value.current_deg,
            target_deg: value.target_deg,
            speed_scale: value.speed_scale,
        }
    }
}
