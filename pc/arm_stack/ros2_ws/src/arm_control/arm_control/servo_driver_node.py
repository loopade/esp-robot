import json
import math
import socket
from collections import deque
from dataclasses import dataclass
from typing import Dict, List, Optional

import rclpy
from rclpy.node import Node
from rcl_interfaces.msg import SetParametersResult
from sensor_msgs.msg import JointState
from std_msgs.msg import Float64

from arm_control.motion_profile import PrecisionMotionProfile


def _as_float_list(value: List[float]) -> List[float]:
    return [float(item) for item in value]


@dataclass
class ServoConfig:
    joint_names: List[str]
    servo_channels: List[int]
    home_position_deg: List[float]
    direction_signs: List[float]
    angle_offsets_deg: List[float]
    min_angle_deg: List[float]
    max_angle_deg: List[float]
    servo_min_deg: List[float]
    servo_max_deg: List[float]
    fixed_output_channels: List[int]
    fixed_output_angles_deg: List[float]


@dataclass
class Esp32UdpConfig:
    host: str
    port: int
    bind_address: str
    bind_port: int


class ServoBackendBase:
    def set_angle(self, channel: int, angle_deg: float) -> None:
        raise NotImplementedError


class SimServoBackend(ServoBackendBase):
    def __init__(self, logger) -> None:
        self._logger = logger
        self._last_angles: Dict[int, float] = {}
        self._logged_startup = False

    def set_angle(self, channel: int, angle_deg: float) -> None:
        self._last_angles[channel] = angle_deg
        if not self._logged_startup:
            self._logger.info("Using simulation backend for servo output.")
            self._logged_startup = True


class ServoKitBackend(ServoBackendBase):
    def __init__(self) -> None:
        from adafruit_servokit import ServoKit

        self._kit = ServoKit(channels=16)

    def set_angle(self, channel: int, angle_deg: float) -> None:
        self._kit.servo[channel].angle = angle_deg


class Esp32UdpBridge:
    def __init__(self, logger, config: Esp32UdpConfig) -> None:
        self._logger = logger
        self._remote = (config.host, config.port)
        self._socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self._socket.bind((config.bind_address, config.bind_port))
        self._socket.setblocking(False)
        self._logger.info(
            f"Using ESP32 UDP backend at {config.host}:{config.port}, local bind {config.bind_address}:{config.bind_port}."
        )

    def close(self) -> None:
        self._socket.close()

    def send_goal(self, positions_deg: List[float], speed_scale: float) -> None:
        self._send(
            {
                "type": "joint_goal",
                "positions_deg": [round(value, 4) for value in positions_deg],
                "speed_scale": round(speed_scale, 4),
            }
        )

    def send_speed_scale(self, speed_scale: float) -> None:
        self._send({"type": "speed_scale", "value": round(speed_scale, 4)})

    def poll_joint_state(self) -> Optional[List[float]]:
        latest_positions: Optional[List[float]] = None
        while True:
            try:
                payload, _ = self._socket.recvfrom(4096)
            except BlockingIOError:
                break
            except OSError as exc:
                self._logger.warning(f"ESP32 UDP receive failed: {exc}")
                break

            try:
                message = json.loads(payload.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                self._logger.warning(f"Ignoring invalid ESP32 UDP payload: {exc}")
                continue

            if message.get("type") != "joint_state":
                continue

            current_deg = message.get("current_deg")
            if not isinstance(current_deg, list):
                continue

            try:
                latest_positions = [float(value) for value in current_deg]
            except (TypeError, ValueError):
                self._logger.warning("Ignoring ESP32 joint_state with non-numeric current_deg.")

        return latest_positions

    def _send(self, payload: Dict[str, object]) -> None:
        try:
            data = json.dumps(payload, separators=(",", ":")).encode("utf-8")
            self._socket.sendto(data, self._remote)
        except OSError as exc:
            self._logger.warning(f"ESP32 UDP send failed: {exc}")


class ServoDriverNode(Node):
    def __init__(self) -> None:
        super().__init__("servo_driver")

        self.declare_parameter("use_sim_backend", True)
        self.declare_parameter("backend_mode", "local")
        self.declare_parameter("command_topic", "/arm/goal_joint_states")
        self.declare_parameter("joint_state_topic", "/joint_states")
        self.declare_parameter("motion_speed_topic", "/arm/motion_speed_scale")
        self.declare_parameter("target_state_topic", "/arm/target_joint_states")
        self.declare_parameter(
            "joint_names",
            ["joint_1", "joint_2", "joint_3", "joint_4", "joint_5", "joint_6"],
        )
        self.declare_parameter("servo_channels", [0, 1, 2, 3, 4, 5])
        self.declare_parameter("home_position_deg", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
        self.declare_parameter("direction_signs", [1, 1, 1, 1, 1, 1])
        self.declare_parameter("angle_offsets_deg", [90.0, 90.0, 90.0, 90.0, 90.0, 90.0])
        self.declare_parameter("min_angle_deg", [-90.0, -90.0, -90.0, -90.0, -90.0, -90.0])
        self.declare_parameter("max_angle_deg", [90.0, 90.0, 90.0, 90.0, 90.0, 90.0])
        self.declare_parameter("servo_min_deg", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
        self.declare_parameter("servo_max_deg", [180.0, 180.0, 180.0, 180.0, 180.0, 180.0])
        self.declare_parameter("fixed_output_channels", [12])
        self.declare_parameter("fixed_output_angles_deg", [90.0])
        self.declare_parameter("update_rate_hz", 30.0)
        self.declare_parameter("max_step_deg", 1.5)
        self.declare_parameter("max_velocity_deg_s", 18.0)
        self.declare_parameter("settle_cycles", 3)
        self.declare_parameter("esp32_udp_host", "192.168.1.88")
        self.declare_parameter("esp32_udp_port", 8888)
        self.declare_parameter("esp32_udp_bind_address", "0.0.0.0")
        self.declare_parameter("esp32_udp_bind_port", 8889)

        self.config = self._load_config()
        self.backend_mode = str(self.get_parameter("backend_mode").value).strip().lower()
        if self.backend_mode not in {"local", "esp32_udp"}:
            self.get_logger().warning(
                f"Unknown backend_mode={self.backend_mode!r}, falling back to local backend."
            )
            self.backend_mode = "local"
        self.command_topic = self.get_parameter("command_topic").get_parameter_value().string_value
        self.joint_state_topic = self.get_parameter("joint_state_topic").get_parameter_value().string_value
        self.target_state_topic = (
            self.get_parameter("target_state_topic").get_parameter_value().string_value
        )
        self.motion_speed_topic = (
            self.get_parameter("motion_speed_topic").get_parameter_value().string_value
        )
        self.update_rate_hz = float(
            self.get_parameter("update_rate_hz").get_parameter_value().double_value
        )
        self.max_step_deg = float(
            self.get_parameter("max_step_deg").get_parameter_value().double_value
        )
        self.max_velocity_deg_s = float(
            self.get_parameter("max_velocity_deg_s").get_parameter_value().double_value
        )
        self.base_max_step_deg = self.max_step_deg
        self.base_max_velocity_deg_s = self.max_velocity_deg_s
        self.speed_scale = 1.0
        self.settle_cycles = int(
            self.get_parameter("settle_cycles").get_parameter_value().integer_value
        )

        self.current_deg = list(self.config.home_position_deg)
        self._trajectory: deque[List[float]] = deque()
        self._settle_remaining = 0
        self.backend: Optional[ServoBackendBase] = None
        self.esp32_bridge: Optional[Esp32UdpBridge] = None

        self.joint_state_pub = self.create_publisher(JointState, self.joint_state_topic, 10)
        self.target_state_pub = self.create_publisher(JointState, self.target_state_topic, 10)
        self.goal_sub = self.create_subscription(
            JointState, self.command_topic, self._on_goal, 10
        )
        self.motion_speed_sub = self.create_subscription(
            Float64, self.motion_speed_topic, self._on_motion_speed, 10
        )
        self.timer = self.create_timer(1.0 / self.update_rate_hz, self._on_timer)
        self.add_on_set_parameters_callback(self._on_parameters_changed)

        if self.backend_mode == "esp32_udp":
            self.esp32_bridge = Esp32UdpBridge(self.get_logger(), self._load_esp32_udp_config())
            self.esp32_bridge.send_speed_scale(self.speed_scale)
            self._publish_joint_state(self.current_deg)
            self.get_logger().info(
                f"Servo driver ready in ESP32 UDP mode. Listening on {self.command_topic}, publishing {self.joint_state_topic}."
            )
        else:
            self.backend = self._build_backend()
            self._apply_positions(self.current_deg)
            self.get_logger().info(
                f"Servo driver ready in local mode. Listening on {self.command_topic}, publishing {self.joint_state_topic}."
            )

    def _load_config(self) -> ServoConfig:
        joint_names = list(self.get_parameter("joint_names").value)
        return ServoConfig(
            joint_names=joint_names,
            servo_channels=[int(item) for item in self.get_parameter("servo_channels").value],
            home_position_deg=_as_float_list(self.get_parameter("home_position_deg").value),
            direction_signs=_as_float_list(self.get_parameter("direction_signs").value),
            angle_offsets_deg=_as_float_list(self.get_parameter("angle_offsets_deg").value),
            min_angle_deg=_as_float_list(self.get_parameter("min_angle_deg").value),
            max_angle_deg=_as_float_list(self.get_parameter("max_angle_deg").value),
            servo_min_deg=_as_float_list(self.get_parameter("servo_min_deg").value),
            servo_max_deg=_as_float_list(self.get_parameter("servo_max_deg").value),
            fixed_output_channels=[
                int(item) for item in self.get_parameter("fixed_output_channels").value
            ],
            fixed_output_angles_deg=_as_float_list(
                self.get_parameter("fixed_output_angles_deg").value
            ),
        )

    def _load_esp32_udp_config(self) -> Esp32UdpConfig:
        return Esp32UdpConfig(
            host=str(self.get_parameter("esp32_udp_host").value),
            port=int(self.get_parameter("esp32_udp_port").value),
            bind_address=str(self.get_parameter("esp32_udp_bind_address").value),
            bind_port=int(self.get_parameter("esp32_udp_bind_port").value),
        )

    def _build_backend(self) -> ServoBackendBase:
        if bool(self.get_parameter("use_sim_backend").value):
            return SimServoBackend(self.get_logger())

        try:
            return ServoKitBackend()
        except Exception as exc:  # pragma: no cover - hardware fallback
            self.get_logger().warning(
                f"Failed to initialize PCA9685 backend ({exc}). Falling back to simulation backend."
            )
            return SimServoBackend(self.get_logger())

    def _on_goal(self, msg: JointState) -> None:
        target_deg = self._extract_goal_deg(msg)
        if target_deg is None:
            return

        if self.backend_mode == "esp32_udp":
            assert self.esp32_bridge is not None
            self.esp32_bridge.send_goal(target_deg, self.speed_scale)
            self._publish_target_state(target_deg)
            return

        current_reference = list(self.current_deg)
        trajectory = PrecisionMotionProfile.plan(
            start_deg=current_reference,
            goal_deg=target_deg,
            max_step_deg=self.max_step_deg,
            max_velocity_deg_s=self.max_velocity_deg_s,
            control_period_sec=1.0 / self.update_rate_hz,
        )
        self._trajectory = deque(trajectory)
        self._settle_remaining = self.settle_cycles
        self._publish_target_state(target_deg)

    def _extract_goal_deg(self, msg: JointState) -> Optional[List[float]]:
        if not msg.position:
            self.get_logger().warning("Ignoring JointState without positions.")
            return None

        if msg.name:
            positions_by_name = {name: pos for name, pos in zip(msg.name, msg.position)}
            if not all(joint in positions_by_name for joint in self.config.joint_names):
                missing = [
                    joint for joint in self.config.joint_names if joint not in positions_by_name
                ]
                self.get_logger().warning(f"Ignoring command. Missing joints: {missing}")
                return None
            raw_deg = [
                math.degrees(positions_by_name[joint]) for joint in self.config.joint_names
            ]
        else:
            if len(msg.position) != len(self.config.joint_names):
                self.get_logger().warning("Ignoring JointState with invalid position length.")
                return None
            raw_deg = [math.degrees(position) for position in msg.position]

        return [
            self._clamp(raw_deg[index], self.config.min_angle_deg[index], self.config.max_angle_deg[index])
            for index in range(len(self.config.joint_names))
        ]

    def _on_timer(self) -> None:
        if self.backend_mode == "esp32_udp":
            assert self.esp32_bridge is not None
            current_deg = self.esp32_bridge.poll_joint_state()
            if current_deg is None:
                return
            if len(current_deg) != len(self.config.joint_names):
                self.get_logger().warning(
                    f"Ignoring ESP32 joint_state with invalid length {len(current_deg)}."
                )
                return

            self.current_deg = [
                self._clamp(
                    current_deg[index],
                    self.config.min_angle_deg[index],
                    self.config.max_angle_deg[index],
                )
                for index in range(len(current_deg))
            ]
            self._publish_joint_state(self.current_deg)
            return

        if self._trajectory:
            self.current_deg = self._trajectory.popleft()
            self._apply_positions(self.current_deg)
            return

        if self._settle_remaining > 0:
            self._apply_positions(self.current_deg)
            self._settle_remaining -= 1

    def _apply_positions(self, joint_positions_deg: List[float]) -> None:
        for index, joint_name in enumerate(self.config.joint_names):
            _ = joint_name
            signed_joint_deg = (
                joint_positions_deg[index] * self.config.direction_signs[index]
            )
            servo_angle_deg = signed_joint_deg + self.config.angle_offsets_deg[index]
            servo_angle_deg = self._clamp(
                servo_angle_deg,
                self.config.servo_min_deg[index],
                self.config.servo_max_deg[index],
            )
            assert self.backend is not None
            self.backend.set_angle(self.config.servo_channels[index], servo_angle_deg)

        for channel, angle_deg in zip(
            self.config.fixed_output_channels, self.config.fixed_output_angles_deg
        ):
            assert self.backend is not None
            self.backend.set_angle(channel, angle_deg)

        self._publish_joint_state(joint_positions_deg)

    def _publish_joint_state(self, joint_positions_deg: List[float]) -> None:
        msg = JointState()
        msg.header.stamp = self.get_clock().now().to_msg()
        msg.name = list(self.config.joint_names)
        msg.position = [math.radians(value) for value in joint_positions_deg]
        self.joint_state_pub.publish(msg)

    def _publish_target_state(self, joint_positions_deg: List[float]) -> None:
        msg = JointState()
        msg.header.stamp = self.get_clock().now().to_msg()
        msg.name = list(self.config.joint_names)
        msg.position = [math.radians(value) for value in joint_positions_deg]
        self.target_state_pub.publish(msg)

    def _on_motion_speed(self, msg: Float64) -> None:
        self.speed_scale = self._clamp(float(msg.data), 0.1, 3.0)

        if self.backend_mode == "esp32_udp":
            assert self.esp32_bridge is not None
            self.esp32_bridge.send_speed_scale(self.speed_scale)
            return

        self.max_step_deg = self.base_max_step_deg * self.speed_scale
        self.max_velocity_deg_s = self.base_max_velocity_deg_s * self.speed_scale

    def _on_parameters_changed(self, params) -> SetParametersResult:
        for param in params:
            if param.name == "max_step_deg":
                self.max_step_deg = float(param.value)
            elif param.name == "max_velocity_deg_s":
                self.max_velocity_deg_s = float(param.value)
            elif param.name == "settle_cycles":
                self.settle_cycles = int(param.value)

        return SetParametersResult(successful=True)

    @staticmethod
    def _clamp(value: float, minimum: float, maximum: float) -> float:
        return min(max(value, minimum), maximum)

    def destroy_node(self):
        if self.esp32_bridge is not None:
            self.esp32_bridge.close()
        return super().destroy_node()


def main(args=None) -> None:
    rclpy.init(args=args)
    node = ServoDriverNode()
    try:
        rclpy.spin(node)
    except KeyboardInterrupt:
        pass
    finally:
        node.destroy_node()
        rclpy.shutdown()
