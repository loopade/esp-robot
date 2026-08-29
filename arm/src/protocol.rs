pub use shared_protocol::ArmCommand;
use serde::Serialize;

use crate::arm::{TelemetryState, JOINT_COUNT};

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
