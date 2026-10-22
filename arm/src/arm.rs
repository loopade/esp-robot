use std::net::SocketAddr;

pub const JOINT_COUNT: usize = 5;
const CONTROL_PERIOD_SEC: f32 = 1.0 / 30.0;
const BASE_MAX_STEP_DEG: f32 = 0.8;
// 100% means a useful 24 deg/s; 300% reaches 72 deg/s.
const BASE_MAX_VELOCITY_DEG_S: f32 = 24.0;
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
    servo_channels: [0, 1, 2, 3, 4],
    home_position_deg: [0.0, 0.0, 0.0, 0.0, 0.0],
    direction_signs: [-1.0, -1.0, 1.0, 1.0, 1.0],
    angle_offsets_deg: [90.0, 90.0, 90.0, 90.0, 90.0],
    min_angle_deg: [-90.0, -90.0, -90.0, -90.0, -90.0],
    max_angle_deg: [90.0, 90.0, 90.0, 90.0, 90.0],
    servo_min_deg: [0.0, 0.0, 0.0, 0.0, 0.0],
    servo_max_deg: [180.0, 180.0, 180.0, 180.0, 180.0],
    servo_pulse_min_us: [500.0, 500.0, 500.0, 500.0, 500.0],
    servo_pulse_max_us: [2500.0, 2500.0, 2500.0, 2500.0, 2500.0],
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
    pub startup_calibrated: bool,
    pub startup_deg: [f32; JOINT_COUNT],
    pub min_angle_deg: [f32; JOINT_COUNT],
    pub max_angle_deg: [f32; JOINT_COUNT],
}

pub struct SharedArmState {
    config: &'static ArmConfig,
    pub current_deg: [f32; JOINT_COUNT],
    target_deg: [f32; JOINT_COUNT],
    speed_scale: f32,
    command_seq: u64,
    motion_plan: Option<MotionPlan>,
    pub last_peer: Option<SocketAddr>,
    startup_calibrated: bool,
    startup_deg: [f32; JOINT_COUNT],
    min_angle_deg: [f32; JOINT_COUNT],
    max_angle_deg: [f32; JOINT_COUNT],
}

impl SharedArmState {
    pub fn new(config: &'static ArmConfig, startup_pose: [f32; JOINT_COUNT], startup_calibrated: bool) -> Self {
        Self {
            config,
            current_deg: startup_pose,
            target_deg: startup_pose,
            speed_scale: 1.0,
            command_seq: 0,
            motion_plan: None,
            last_peer: None,
            startup_calibrated,
            startup_deg: startup_pose,
            min_angle_deg: config.min_angle_deg,
            max_angle_deg: config.max_angle_deg,
        }
    }

    pub fn set_startup_pose(&mut self, pose: [f32; JOINT_COUNT]) {
        self.startup_deg = self.clamp_pose(pose);
        self.startup_calibrated = true;
    }

    pub fn set_joint_limits(&mut self, minimum: [f32; JOINT_COUNT], maximum: [f32; JOINT_COUNT]) -> bool {
        if (0..JOINT_COUNT).any(|i| !minimum[i].is_finite() || !maximum[i].is_finite() || minimum[i] >= maximum[i]) { return false; }
        self.min_angle_deg = minimum;
        self.max_angle_deg = maximum;
        self.startup_deg = self.clamp_pose(self.startup_deg);
        self.target_deg = self.clamp_pose(self.target_deg);
        self.current_deg = self.clamp_pose(self.current_deg);
        self.motion_plan = Some(MotionPlan::new(self.current_deg, self.target_deg, self.speed_scale));
        true
    }

    fn clamp_pose(&self, pose: [f32; JOINT_COUNT]) -> [f32; JOINT_COUNT] {
        core::array::from_fn(|i| clamp(pose[i], self.min_angle_deg[i], self.max_angle_deg[i]))
    }

    pub fn clamp_to_joint_limits(&self, pose: [f32; JOINT_COUNT]) -> [f32; JOINT_COUNT] { self.clamp_pose(pose) }

    pub fn apply_goal(
        &mut self,
        positions_deg: [f32; JOINT_COUNT],
        speed_scale: Option<f32>,
        seq: Option<u64>,
    ) {
        let previous_speed_scale = self.speed_scale;
        if let Some(value) = speed_scale {
            self.speed_scale = clamp(value, MIN_SPEED_SCALE, MAX_SPEED_SCALE);
        }

        let mut target = [0.0; JOINT_COUNT];
        for index in 0..JOINT_COUNT {
            target[index] = clamp(
                positions_deg[index],
                self.min_angle_deg[index],
                self.max_angle_deg[index],
            );
        }

        // Web real-time mode repeats goals periodically. Keep the in-flight
        // trajectory when a repeated goal has not changed.
        if target == self.target_deg {
            if self.speed_scale != previous_speed_scale && self.motion_plan.is_some() {
                self.motion_plan = Some(MotionPlan::new(self.current_deg, target, self.speed_scale));
            }
            return;
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
            startup_calibrated: self.startup_calibrated,
            startup_deg: self.startup_deg,
            min_angle_deg: self.min_angle_deg,
            max_angle_deg: self.max_angle_deg,
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
