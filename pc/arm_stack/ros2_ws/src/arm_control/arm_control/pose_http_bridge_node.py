import json
import math
import threading
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import List, Optional

import rclpy
from rclpy.node import Node
from sensor_msgs.msg import JointState
from std_msgs.msg import Float64


class PoseHttpBridgeNode(Node):
    def __init__(self) -> None:
        super().__init__("pose_http_bridge")

        self.declare_parameter(
            "joint_names",
            ["joint_1", "joint_2", "joint_3", "joint_4", "joint_5", "joint_6"],
        )
        self.declare_parameter("command_topic", "/arm/goal_joint_states")
        self.declare_parameter("joint_state_topic", "/joint_states")
        self.declare_parameter("motion_speed_topic", "/arm/motion_speed_scale")
        self.declare_parameter("listen_host", "0.0.0.0")
        self.declare_parameter("listen_port", 8765)
        self.declare_parameter("max_step_deg", 0.8)
        self.declare_parameter("max_velocity_deg_s", 8.0)
        self.declare_parameter(
            "target_min_deg",
            [-120.0, -180.0, -180.0, -180.0, -180.0, -180.0],
        )
        self.declare_parameter(
            "target_max_deg",
            [120.0, 180.0, 180.0, 180.0, 180.0, 180.0],
        )

        self.joint_names = [str(item) for item in self.get_parameter("joint_names").value]
        self.command_topic = str(self.get_parameter("command_topic").value)
        self.joint_state_topic = str(self.get_parameter("joint_state_topic").value)
        self.motion_speed_topic = str(self.get_parameter("motion_speed_topic").value)
        self.listen_host = str(self.get_parameter("listen_host").value)
        self.listen_port = int(self.get_parameter("listen_port").value)
        self.base_max_step_deg = float(self.get_parameter("max_step_deg").value)
        self.base_max_velocity_deg_s = float(self.get_parameter("max_velocity_deg_s").value)
        self.target_min_deg = [float(item) for item in self.get_parameter("target_min_deg").value]
        self.target_max_deg = [float(item) for item in self.get_parameter("target_max_deg").value]
        self.current_speed_scale = 1.0

        self._state_lock = threading.Lock()
        self._latest_positions_deg: List[float] = [0.0] * len(self.joint_names)
        self._latest_target_deg: List[float] = [0.0] * len(self.joint_names)
        self._has_joint_state = False
        self._has_target_state = False

        self.goal_publisher = self.create_publisher(JointState, self.command_topic, 10)
        self.motion_speed_publisher = self.create_publisher(Float64, self.motion_speed_topic, 10)
        self.joint_state_subscription = self.create_subscription(
            JointState, self.joint_state_topic, self._on_joint_state, 10
        )

        self._http_server = ThreadingHTTPServer(
            (self.listen_host, self.listen_port), self._build_handler()
        )
        self._http_thread = threading.Thread(
            target=self._http_server.serve_forever, daemon=True
        )
        self._http_thread.start()

        self.get_logger().info(
            f"Pose HTTP bridge ready on http://{self.listen_host}:{self.listen_port}"
        )

    def _build_handler(self):
        node = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, format, *args) -> None:  # noqa: A003
                return

            def do_GET(self) -> None:  # noqa: N802
                if self.path == "/health":
                    self._send_json(
                        HTTPStatus.OK,
                        {
                            "ok": True,
                            "joint_names": node.joint_names,
                            "has_joint_state": node._has_joint_state,
                            "has_target_state": node._has_target_state,
                        },
                    )
                    return

                if self.path == "/joint_state":
                    positions_deg = node.get_latest_positions_deg()
                    self._send_json(
                        HTTPStatus.OK,
                        {
                            "ok": True,
                            "joint_names": node.joint_names,
                            "positions_deg": positions_deg,
                            "has_joint_state": node._has_joint_state,
                        },
                    )
                    return

                if self.path == "/target_state":
                    target_deg = node.get_latest_target_deg()
                    self._send_json(
                        HTTPStatus.OK,
                        {
                            "ok": True,
                            "joint_names": node.joint_names,
                            "positions_deg": target_deg,
                            "target_min_deg": node.target_min_deg,
                            "target_max_deg": node.target_max_deg,
                            "has_target_state": node._has_target_state,
                        },
                    )
                    return

                if self.path == "/motion_config":
                    self._send_json(
                        HTTPStatus.OK,
                        {
                            "ok": True,
                            "speed_scale": node.current_speed_scale,
                            "max_step_deg": node.base_max_step_deg * node.current_speed_scale,
                            "max_velocity_deg_s": node.base_max_velocity_deg_s * node.current_speed_scale,
                        },
                    )
                    return

                self._send_json(HTTPStatus.NOT_FOUND, {"ok": False, "error": "not found"})

            def do_POST(self) -> None:  # noqa: N802
                content_length = int(self.headers.get("Content-Length", "0"))
                raw_body = self.rfile.read(content_length)

                try:
                    payload = json.loads(raw_body.decode("utf-8"))
                except json.JSONDecodeError:
                    self._send_json(
                        HTTPStatus.BAD_REQUEST, {"ok": False, "error": "invalid json"}
                    )
                    return

                if self.path == "/goal":
                    positions_deg = payload.get("positions_deg")
                    if not isinstance(positions_deg, list) or len(positions_deg) != len(
                        node.joint_names
                    ):
                        self._send_json(
                            HTTPStatus.BAD_REQUEST,
                            {"ok": False, "error": "positions_deg must contain 6 values"},
                        )
                        return

                    try:
                        positions_deg = [float(value) for value in positions_deg]
                    except (TypeError, ValueError):
                        self._send_json(
                            HTTPStatus.BAD_REQUEST,
                            {"ok": False, "error": "positions_deg must be numeric"},
                        )
                        return

                    positions_deg = node.publish_goal(positions_deg)
                    self._send_json(
                        HTTPStatus.OK,
                        {
                            "ok": True,
                            "joint_names": node.joint_names,
                            "positions_deg": positions_deg,
                        },
                    )
                    return

                if self.path == "/motion_config":
                    speed_scale = payload.get("speed_scale")
                    if speed_scale is None:
                        self._send_json(
                            HTTPStatus.BAD_REQUEST,
                            {"ok": False, "error": "speed_scale is required"},
                        )
                        return

                    try:
                        speed_scale = float(speed_scale)
                    except (TypeError, ValueError):
                        self._send_json(
                            HTTPStatus.BAD_REQUEST,
                            {"ok": False, "error": "speed_scale must be numeric"},
                        )
                        return

                    ok, message = node.update_motion_config(speed_scale)
                    status = HTTPStatus.OK if ok else HTTPStatus.BAD_GATEWAY
                    self._send_json(
                        status,
                        {
                            "ok": ok,
                            "message": message,
                            "speed_scale": node.current_speed_scale,
                            "max_step_deg": node.base_max_step_deg * node.current_speed_scale,
                            "max_velocity_deg_s": node.base_max_velocity_deg_s * node.current_speed_scale,
                        },
                    )
                    return

                self._send_json(HTTPStatus.NOT_FOUND, {"ok": False, "error": "not found"})

            def _send_json(self, status: HTTPStatus, payload: dict) -> None:
                data = json.dumps(payload).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        return Handler

    def _on_joint_state(self, msg: JointState) -> None:
        if not msg.name or not msg.position:
            return

        positions_by_name = {name: pos for name, pos in zip(msg.name, msg.position)}
        if not all(joint in positions_by_name for joint in self.joint_names):
            return

        with self._state_lock:
            self._latest_positions_deg = [
                math.degrees(positions_by_name[joint]) for joint in self.joint_names
            ]
            self._has_joint_state = True
            if not self._has_target_state:
                self._latest_target_deg = list(self._latest_positions_deg)
                self._has_target_state = True

    def get_latest_positions_deg(self) -> List[float]:
        with self._state_lock:
            return list(self._latest_positions_deg)

    def get_latest_target_deg(self) -> List[float]:
        with self._state_lock:
            return list(self._latest_target_deg)

    def publish_goal(self, positions_deg: List[float]) -> List[float]:
        clamped_positions_deg = self._clamp_positions(positions_deg)

        with self._state_lock:
            self._latest_target_deg = list(clamped_positions_deg)
            self._has_target_state = True

        msg = JointState()
        msg.header.stamp = self.get_clock().now().to_msg()
        msg.name = list(self.joint_names)
        msg.position = [math.radians(value) for value in clamped_positions_deg]
        self.goal_publisher.publish(msg)
        return clamped_positions_deg

    def update_motion_config(self, speed_scale: float) -> tuple[bool, str]:
        clamped_scale = max(0.1, min(3.0, speed_scale))
        msg = Float64()
        msg.data = clamped_scale
        self.motion_speed_publisher.publish(msg)
        self.current_speed_scale = clamped_scale
        return True, "motion config updated"

    def destroy_node(self) -> bool:
        if hasattr(self, "_http_server") and self._http_server is not None:
            self._http_server.shutdown()
            self._http_server.server_close()
        return super().destroy_node()

    def _clamp_positions(self, positions_deg: List[float]) -> List[float]:
        return [
            max(self.target_min_deg[index], min(self.target_max_deg[index], float(value)))
            for index, value in enumerate(positions_deg)
        ]


def main(args=None) -> None:
    rclpy.init(args=args)
    node = PoseHttpBridgeNode()
    try:
        rclpy.spin(node)
    except KeyboardInterrupt:
        pass
    finally:
        node.destroy_node()
        rclpy.shutdown()
