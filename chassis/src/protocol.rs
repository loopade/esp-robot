use serde::Serialize;

use crate::arm::{TelemetryState, JOINT_COUNT};

#[derive(Debug, Serialize)]
pub struct JointStatePacket {
    pub r#type: &'static str,
    pub seq: u64,
    pub current_deg: [f32; JOINT_COUNT],
    pub target_deg: [f32; JOINT_COUNT],
    pub speed_scale: f32,
    pub startup_calibrated: bool,
    pub startup_deg: [f32; JOINT_COUNT],
    pub min_angle_deg: [f32; JOINT_COUNT],
    pub max_angle_deg: [f32; JOINT_COUNT],
}

impl From<TelemetryState> for JointStatePacket {
    fn from(value: TelemetryState) -> Self {
        Self {
            r#type: "joint_state",
            seq: value.seq,
            current_deg: value.current_deg,
            target_deg: value.target_deg,
            speed_scale: value.speed_scale,
            startup_calibrated: value.startup_calibrated,
            startup_deg: value.startup_deg,
            min_angle_deg: value.min_angle_deg,
            max_angle_deg: value.max_angle_deg,
        }
    }
}
