use std::net::SocketAddr;

pub const JOINT_COUNT: usize = 6;
const CONTROL_PERIOD_SEC: f32 = 1.0 / 30.0;
const BASE_MAX_STEP_DEG: f32 = 0.8;
const BASE_MAX_VELOCITY_DEG_S: f32 = 8.0;
const MIN_SPEED_SCALE: f32 = 0.1;
const MAX_SPEED_SCALE: f32 = 3.0;

#[derive(Copy, Clone)]
pub struct ArmConfig {
    pub servo_channels: [u8; JOINT_COUNT],
    pub home_position_deg: [f32; JOINT_COUNT],
    pub direction_signs: [f32; JOINT_COUNT],
    pub angle_offsets_deg: [f32; JOINT_COUNT],
    pub min_angle_deg: [f32; JOINT_COUNT],
    pub max_angle_deg: [f32; JOINT_COUNT],
    pub servo_min_deg: [f32; JOINT_COUNT],
    pub servo_max_deg: [f32; JOINT_COUNT],
    pub servo_pulse_min_us: [f32; JOINT_COUNT],
    pub servo_pulse_max_us: [f32; JOINT_COUNT],
    pub fixed_output_channels: [u8; 1],
    pub fixed_output_angles_deg: [f32; 1],
}

pub const ARM_CONFIG: ArmConfig = ArmConfig {
    servo_channels: [0, 1, 2, 3, 4, 5],
    home_position_deg: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    direction_signs: [-1.0, -1.0, 1.0, 1.0, 1.0, 1.0],
    angle_offsets_deg: [90.0, 90.0, 90.0, 90.0, 90.0, 90.0],
    min_angle_deg: [-180.0, -180.0, -180.0, -180.0, -180.0, -180.0],
    max_angle_deg: [180.0, 180.0, 180.0, 180.0, 180.0, 180.0],
    servo_min_deg: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    servo_max_deg: [180.0, 180.0, 180.0, 180.0, 180.0, 180.0],
    servo_pulse_min_us: [500.0, 500.0, 500.0, 500.0, 500.0, 500.0],
    servo_pulse_max_us: [2500.0, 2500.0, 2500.0, 2500.0, 2500.0, 2500.0],
    fixed_output_channels: [12],
    fixed_output_angles_deg: [90.0],
};

#[derive(Clone, Copy, Debug)]
struct MotionPlan {
    start_deg: [f32; JOINT_COUNT],
    goal_deg: [f32; JOINT_COUNT],
    step_index: u32,
    steps_total: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct TelemetryState {
    pub seq: u64,
    pub current_deg: [f32; JOINT_COUNT],
    pub target_deg: [f32; JOINT_COUNT],
    pub speed_scale: f32,
}

pub struct SharedArmState {
    config: &'static ArmConfig,
    pub current_deg: [f32; JOINT_COUNT],
    target_deg: [f32; JOINT_COUNT],
    speed_scale: f32,
    command_seq: u64,
    motion_plan: Option<MotionPlan>,
    pub last_peer: Option<SocketAddr>,
}

impl SharedArmState {
    pub fn new(config: &'static ArmConfig) -> Self {
        Self {
            config,
            current_deg: config.home_position_deg,
            target_deg: config.home_position_deg,
            speed_scale: 1.0,
            command_seq: 0,
            motion_plan: None,
            last_peer: None,
        }
    }

    pub fn apply_goal(
        &mut self,
        positions_deg: [f32; JOINT_COUNT],
        speed_scale: Option<f32>,
        seq: Option<u64>,
    ) {
        if let Some(value) = speed_scale {
            self.speed_scale = clamp(value, MIN_SPEED_SCALE, MAX_SPEED_SCALE);
        }

        let mut target = [0.0; JOINT_COUNT];
        for index in 0..JOINT_COUNT {
            target[index] = clamp(
                positions_deg[index],
                self.config.min_angle_deg[index],
                self.config.max_angle_deg[index],
            );
        }

        self.target_deg = target;
        self.command_seq = seq.unwrap_or_else(|| self.command_seq.wrapping_add(1));

        let plan = MotionPlan::new(self.current_deg, self.target_deg, self.speed_scale);
        if plan.steps_total == 0 {
            self.current_deg = self.target_deg;
            self.motion_plan = None;
        } else {
            self.motion_plan = Some(plan);
        }
    }

    pub fn set_speed_scale(&mut self, value: f32) {
        self.speed_scale = clamp(value, MIN_SPEED_SCALE, MAX_SPEED_SCALE);
        if self.motion_plan.is_some() {
            self.motion_plan = Some(MotionPlan::new(
                self.current_deg,
                self.target_deg,
                self.speed_scale,
            ));
        }
    }

    pub fn set_last_peer(&mut self, peer: SocketAddr) {
        self.last_peer = Some(peer);
    }

    pub fn step(&mut self) -> bool {
        let Some(mut plan) = self.motion_plan else {
            return false;
        };

        if let Some(next) = plan.next() {
            self.current_deg = next;
        }

        if plan.is_complete() {
            self.current_deg = self.target_deg;
            self.motion_plan = None;
        } else {
            self.motion_plan = Some(plan);
        }

        true
    }

    pub fn telemetry(&self) -> TelemetryState {
        TelemetryState {
            seq: self.command_seq,
            current_deg: self.current_deg,
            target_deg: self.target_deg,
            speed_scale: self.speed_scale,
        }
    }
}

impl ArmConfig {
    pub fn joint_to_servo_angle(&self, index: usize, joint_deg: f32) -> f32 {
        let signed = joint_deg * self.direction_signs[index];
        clamp(
            signed + self.angle_offsets_deg[index],
            self.servo_min_deg[index],
            self.servo_max_deg[index],
        )
    }
}

impl MotionPlan {
    fn new(start_deg: [f32; JOINT_COUNT], goal_deg: [f32; JOINT_COUNT], speed_scale: f32) -> Self {
        let mut max_travel = 0.0_f32;
        for index in 0..JOINT_COUNT {
            max_travel = max_travel.max((goal_deg[index] - start_deg[index]).abs());
        }

        if max_travel <= 1e-6 {
            return Self {
                start_deg,
                goal_deg,
                step_index: 0,
                steps_total: 0,
            };
        }

        let max_step_deg = (BASE_MAX_STEP_DEG * speed_scale).max(1e-6);
        let max_velocity_deg_s = (BASE_MAX_VELOCITY_DEG_S * speed_scale).max(1e-6);
        let step_limited = (max_travel / max_step_deg).ceil().max(1.0);
        let duration_sec = (max_travel / max_velocity_deg_s).max(CONTROL_PERIOD_SEC);
        let velocity_limited = (duration_sec / CONTROL_PERIOD_SEC).ceil().max(1.0);
        let steps_total = step_limited.max(velocity_limited) as u32;

        Self {
            start_deg,
            goal_deg,
            step_index: 0,
            steps_total,
        }
    }

    fn next(&mut self) -> Option<[f32; JOINT_COUNT]> {
        if self.steps_total == 0 || self.step_index >= self.steps_total {
            return None;
        }

        self.step_index += 1;
        let ratio = self.step_index as f32 / self.steps_total as f32;
        let eased = 0.5 - 0.5 * (core::f32::consts::PI * ratio).cos();
        let mut positions = [0.0; JOINT_COUNT];
        for index in 0..JOINT_COUNT {
            positions[index] =
                self.start_deg[index] + (self.goal_deg[index] - self.start_deg[index]) * eased;
        }

        Some(positions)
    }

    fn is_complete(&self) -> bool {
        self.steps_total == 0 || self.step_index >= self.steps_total
    }
}

fn clamp(value: f32, minimum: f32, maximum: f32) -> f32 {
    value.max(minimum).min(maximum)
}
