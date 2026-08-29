import json
import math
import os
import sys
import time
import urllib.error
import urllib.request
from typing import Callable, List, Optional

import rclpy
from python_qt_binding.QtCore import QTimer, Qt
from python_qt_binding.QtGui import QFont
from python_qt_binding.QtWidgets import (
    QApplication,
    QCheckBox,
    QDoubleSpinBox,
    QGridLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QSlider,
    QVBoxLayout,
    QWidget,
)
from rclpy.node import Node
from sensor_msgs.msg import JointState


JOINT_NAMES = [f"joint_{index}" for index in range(1, 7)]
JOINT_LABELS = ["底座", "大臂", "小臂", "腕部", "爪旋转", "爪夹"]


class ArmPoseDebugBridge(Node):
    def __init__(self) -> None:
        super().__init__("pose_debug_panel")

        self.preview_publisher = self.create_publisher(JointState, "/debug_joint_states", 10)

        self.http_host = os.environ.get("ARM_HTTP_HOST", "127.0.0.1")
        self.http_port = int(os.environ.get("ARM_HTTP_PORT", "8765"))
        self.http_base_url = f"http://{self.http_host}:{self.http_port}"
        self._http_opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

        self.latest_actual_deg: Optional[List[float]] = None
        self.latest_target_deg: Optional[List[float]] = None
        self.target_min_deg: Optional[List[float]] = None
        self.target_max_deg: Optional[List[float]] = None
        self.last_http_error: Optional[str] = None

    def publish_goal(self, positions_deg: List[float]) -> tuple[bool, str]:
        ok, message, payload = self._post_json("/goal", {"positions_deg": positions_deg})
        if ok and isinstance(payload.get("positions_deg"), list):
            self.latest_target_deg = [float(value) for value in payload["positions_deg"]]
        return ok, message

    def update_speed_scale(self, speed_scale: float) -> tuple[bool, str]:
        ok, message, _ = self._post_json("/motion_config", {"speed_scale": speed_scale})
        return ok, message

    def publish_preview(self, positions_deg: List[float]) -> None:
        self.preview_publisher.publish(self._build_joint_state(positions_deg))

    def fetch_remote_joint_state(self) -> tuple[bool, Optional[str]]:
        ok, payload_or_error = self._get_json("/joint_state")
        if not ok:
            return False, str(payload_or_error)

        payload = payload_or_error
        positions_deg = payload.get("positions_deg")
        if isinstance(positions_deg, list) and len(positions_deg) == len(JOINT_NAMES):
            self.latest_actual_deg = [float(value) for value in positions_deg]
            self.last_http_error = None
            return True, None

        return False, "joint_state payload invalid"

    def fetch_remote_target_state(self) -> tuple[bool, Optional[str]]:
        ok, payload_or_error = self._get_json("/target_state")
        if not ok:
            return False, str(payload_or_error)

        payload = payload_or_error
        positions_deg = payload.get("positions_deg")
        target_min_deg = payload.get("target_min_deg")
        target_max_deg = payload.get("target_max_deg")
        if (
            isinstance(positions_deg, list)
            and isinstance(target_min_deg, list)
            and isinstance(target_max_deg, list)
            and len(positions_deg) == len(JOINT_NAMES)
            and len(target_min_deg) == len(JOINT_NAMES)
            and len(target_max_deg) == len(JOINT_NAMES)
        ):
            self.latest_target_deg = [float(value) for value in positions_deg]
            self.target_min_deg = [float(value) for value in target_min_deg]
            self.target_max_deg = [float(value) for value in target_max_deg]
            self.last_http_error = None
            return True, None

        return False, "target_state payload invalid"

    def _get_json(self, path: str) -> tuple[bool, dict | str]:
        try:
            with self._http_opener.open(f"{self.http_base_url}{path}", timeout=0.5) as response:
                payload = json.loads(response.read().decode("utf-8"))
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            self.last_http_error = str(exc)
            return False, str(exc)

        return True, payload

    def _post_json(self, path: str, payload: dict) -> tuple[bool, str, dict]:
        request = urllib.request.Request(
            url=f"{self.http_base_url}{path}",
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )

        try:
            with self._http_opener.open(request, timeout=1.0) as response:
                body = json.loads(response.read().decode("utf-8"))
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            self.last_http_error = str(exc)
            return False, str(exc), {}

        self.last_http_error = None
        if body.get("ok"):
            return True, body.get("message", "ok"), body
        return False, body.get("message", "request failed"), body

    def _build_joint_state(self, positions_deg: List[float]) -> JointState:
        msg = JointState()
        msg.header.stamp = self.get_clock().now().to_msg()
        msg.name = list(JOINT_NAMES)
        msg.position = [math.radians(value) for value in positions_deg]
        return msg


class JointRow:
    def __init__(
        self,
        label: str,
        row_index: int,
        layout: QGridLayout,
        on_changed: Callable[[int, float], None],
    ) -> None:
        self._on_changed = on_changed
        self._suppress_events = False

        self.name_label = QLabel(label)
        self.actual_label = QLabel("--")
        self.actual_label.setAlignment(Qt.AlignRight | Qt.AlignVCenter)

        self.minus_button = QPushButton("-5 deg")
        self.plus_button = QPushButton("+5 deg")

        self.slider = QSlider(Qt.Horizontal)
        self.slider.setRange(-180, 180)
        self.slider.setSingleStep(1)
        self.slider.setPageStep(5)

        self.spin = QDoubleSpinBox()
        self.spin.setRange(-180.0, 180.0)
        self.spin.setDecimals(1)
        self.spin.setSingleStep(1.0)
        self.spin.setSuffix(" deg")

        layout.addWidget(self.name_label, row_index, 0)
        layout.addWidget(self.actual_label, row_index, 1)
        layout.addWidget(self.minus_button, row_index, 2)
        layout.addWidget(self.slider, row_index, 3)
        layout.addWidget(self.plus_button, row_index, 4)
        layout.addWidget(self.spin, row_index, 5)

    def connect(self, joint_index: int) -> None:
        self.slider.valueChanged.connect(
            lambda value, index=joint_index: self._handle_slider_change(index, value)
        )
        self.spin.valueChanged.connect(
            lambda value, index=joint_index: self._handle_spin_change(index, value)
        )
        self.minus_button.clicked.connect(
            lambda _, index=joint_index: self._handle_nudge(index, -5.0)
        )
        self.plus_button.clicked.connect(
            lambda _, index=joint_index: self._handle_nudge(index, 5.0)
        )

    def set_target_range(self, minimum_deg: float, maximum_deg: float) -> None:
        minimum = int(round(minimum_deg))
        maximum = int(round(maximum_deg))
        self._suppress_events = True
        self.slider.setRange(minimum, maximum)
        self.spin.setRange(float(minimum), float(maximum))
        self._suppress_events = False

    def set_target_deg(self, value: float) -> None:
        clamped = min(max(value, self.spin.minimum()), self.spin.maximum())
        self._suppress_events = True
        self.slider.setValue(int(round(clamped)))
        self.spin.setValue(float(clamped))
        self._suppress_events = False

    def set_actual_deg(self, value: Optional[float]) -> None:
        if value is None:
            self.actual_label.setText("--")
            return
        self.actual_label.setText(f"{value:6.1f} deg")

    def current_target_deg(self) -> float:
        return float(self.spin.value())

    def _handle_slider_change(self, joint_index: int, value: int) -> None:
        if self._suppress_events:
            return
        self._suppress_events = True
        self.spin.setValue(float(value))
        self._suppress_events = False
        self._on_changed(joint_index, float(value))

    def _handle_spin_change(self, joint_index: int, value: float) -> None:
        if self._suppress_events:
            return
        rounded = int(round(value))
        self._suppress_events = True
        self.slider.setValue(rounded)
        self._suppress_events = False
        self._on_changed(joint_index, float(value))

    def _handle_nudge(self, joint_index: int, delta: float) -> None:
        self.set_target_deg(self.current_target_deg() + delta)
        self._on_changed(joint_index, self.current_target_deg())


class ArmPoseDebugPanel(QWidget):
    def __init__(self, bridge: ArmPoseDebugBridge) -> None:
        super().__init__()
        self._bridge = bridge
        self._target_deg = [0.0] * len(JOINT_NAMES)
        self._has_synced_actual = False
        self._last_live_sent_deg: Optional[List[float]] = None
        self._speed_scale = 1.0
        self._last_local_target_edit_time = 0.0
        self._last_remote_target_applied: Optional[List[float]] = None
        self._target_limits_applied = False

        self.setWindowTitle("Arm Debug Panel")
        self.resize(1040, 520)

        root_layout = QVBoxLayout()

        status_group = QGroupBox("状态")
        status_layout = QHBoxLayout()
        self.status_label = QLabel("等待机械臂姿态回读...")
        status_layout.addWidget(self.status_label)
        status_group.setLayout(status_layout)
        root_layout.addWidget(status_group)

        motion_group = QGroupBox("运动参数")
        motion_layout = QHBoxLayout()
        self.speed_title_label = QLabel("速度比例")
        self.speed_value_label = QLabel("1.0x")
        self.speed_slider = QSlider(Qt.Horizontal)
        self.speed_slider.setRange(10, 300)
        self.speed_slider.setSingleStep(5)
        self.speed_slider.setPageStep(10)
        self.speed_slider.setValue(100)
        self.live_apply_checkbox = QCheckBox("实时生效")
        self.live_apply_checkbox.setChecked(True)

        motion_layout.addWidget(self.speed_title_label)
        motion_layout.addWidget(self.speed_slider)
        motion_layout.addWidget(self.speed_value_label)
        motion_layout.addWidget(self.live_apply_checkbox)
        motion_group.setLayout(motion_layout)
        root_layout.addWidget(motion_group)

        control_group = QGroupBox("目标姿态")
        control_layout = QGridLayout()
        control_layout.addWidget(QLabel("关节"), 0, 0)
        control_layout.addWidget(QLabel("当前实机"), 0, 1)
        control_layout.addWidget(QLabel("微调"), 0, 2)
        control_layout.addWidget(QLabel("目标"), 0, 3)
        control_layout.addWidget(QLabel("微调"), 0, 4)
        control_layout.addWidget(QLabel("精确值"), 0, 5)

        self._rows: List[JointRow] = []
        for index, label in enumerate(JOINT_LABELS, start=1):
            row = JointRow(label, index, control_layout, self._on_target_changed)
            row.connect(index - 1)
            self._rows.append(row)

        control_group.setLayout(control_layout)
        root_layout.addWidget(control_group)

        button_layout = QHBoxLayout()
        self.sync_button = QPushButton("同步当前姿态")
        self.zero_button = QPushButton("设为零位姿态")
        self.apply_button = QPushButton("发送目标到机械臂")
        self.hold_button = QPushButton("保持当前姿态")

        self.sync_button.clicked.connect(self._sync_actual_to_target)
        self.zero_button.clicked.connect(self._set_zero_pose)
        self.apply_button.clicked.connect(self._apply_target)
        self.hold_button.clicked.connect(self._hold_current_pose)
        self.live_apply_checkbox.toggled.connect(self._on_live_mode_toggled)
        self.speed_slider.valueChanged.connect(self._on_speed_slider_changed)

        button_layout.addWidget(self.sync_button)
        button_layout.addWidget(self.zero_button)
        button_layout.addWidget(self.apply_button)
        button_layout.addWidget(self.hold_button)
        root_layout.addLayout(button_layout)
        self.setLayout(root_layout)

        self._ros_timer = QTimer(self)
        self._ros_timer.timeout.connect(self._spin_ros_once)
        self._ros_timer.start(30)

        self._ui_timer = QTimer(self)
        self._ui_timer.timeout.connect(self._refresh_ui)
        self._ui_timer.start(100)

        self._actual_timer = QTimer(self)
        self._actual_timer.timeout.connect(self._pull_actual_joint_state)
        self._actual_timer.start(400)

        self._target_timer = QTimer(self)
        self._target_timer.timeout.connect(self._pull_remote_target_state)
        self._target_timer.start(200)

        self._preview_timer = QTimer(self)
        self._preview_timer.timeout.connect(self._publish_preview)
        self._preview_timer.start(150)

        self._live_send_timer = QTimer(self)
        self._live_send_timer.setSingleShot(True)
        self._live_send_timer.timeout.connect(self._apply_target_live)

        self._speed_timer = QTimer(self)
        self._speed_timer.setSingleShot(True)
        self._speed_timer.timeout.connect(self._apply_speed_scale)

    def _spin_ros_once(self) -> None:
        rclpy.spin_once(self._bridge, timeout_sec=0.0)

    def _refresh_ui(self) -> None:
        actual_deg = self._bridge.latest_actual_deg
        if actual_deg is None:
            if self._bridge.last_http_error:
                self.status_label.setText(f"等待机械臂姿态回读... {self._bridge.last_http_error}")
            return

        for row, value in zip(self._rows, actual_deg):
            row.set_actual_deg(value)

        self._apply_target_limits_if_available()

        if not self._has_synced_actual:
            initial_target = self._bridge.latest_target_deg or actual_deg
            self._set_target_pose(initial_target)
            self._last_remote_target_applied = list(initial_target)
            self._has_synced_actual = True

        self._sync_remote_target_if_idle(self._bridge.latest_target_deg)

        if self.live_apply_checkbox.isChecked():
            self.status_label.setText("已连接机械臂，实时生效已开启。")
        else:
            self.status_label.setText("已连接机械臂，当前为手动发送模式。")

    def _pull_actual_joint_state(self) -> None:
        ok, error = self._bridge.fetch_remote_joint_state()
        if not ok and error:
            self.status_label.setText(f"机械臂连接异常: {error}")

    def _pull_remote_target_state(self) -> None:
        ok, error = self._bridge.fetch_remote_target_state()
        if not ok and error:
            self.status_label.setText(f"目标状态同步异常: {error}")

    def _publish_preview(self) -> None:
        self._bridge.publish_preview(self._target_deg)

    def _set_target_pose(self, positions_deg: List[float]) -> None:
        self._target_deg = [float(value) for value in positions_deg]
        for row, value in zip(self._rows, self._target_deg):
            row.set_target_deg(value)

    def _on_target_changed(self, joint_index: int, value_deg: float) -> None:
        self._target_deg[joint_index] = float(value_deg)
        self._last_local_target_edit_time = time.monotonic()
        self._live_send_timer.start(120)

    def _on_live_mode_toggled(self, enabled: bool) -> None:
        if enabled:
            self.status_label.setText("实时生效已开启。拖动结束后会自动发送。")
            self._live_send_timer.start(120)
            return

        self._live_send_timer.stop()
        self.status_label.setText("实时生效已关闭。需要手动点击“发送目标到机械臂”。")

    def _on_speed_slider_changed(self, value: int) -> None:
        self._speed_scale = value / 100.0
        self.speed_value_label.setText(f"{self._speed_scale:.1f}x")
        self._speed_timer.start(150)

    def _apply_speed_scale(self) -> None:
        ok, message = self._bridge.update_speed_scale(self._speed_scale)
        if ok:
            self.status_label.setText(f"速度比例已更新为 {self._speed_scale:.1f}x")
            return
        self.status_label.setText(f"速度更新失败: {message}")

    def _sync_actual_to_target(self) -> None:
        if self._bridge.latest_actual_deg is None:
            self.status_label.setText("还没有收到机械臂姿态，暂时不能同步当前姿态。")
            return

        self._set_target_pose(self._bridge.latest_actual_deg)
        self.status_label.setText("目标姿态已同步到当前实机角度。")

    def _set_zero_pose(self) -> None:
        self._set_target_pose([0.0] * len(JOINT_NAMES))
        self.status_label.setText("目标姿态已设置为零位姿态。")
        self._live_send_timer.start(120)

    def _apply_target_live(self) -> None:
        if self._last_live_sent_deg == self._target_deg:
            return
        self._send_target(live_mode=True)

    def _apply_target(self) -> None:
        self._send_target(live_mode=False)

    def _send_target(self, live_mode: bool) -> None:
        ok, message = self._bridge.publish_goal(self._target_deg)
        if ok:
            if self._bridge.latest_target_deg is not None:
                self._set_target_pose(self._bridge.latest_target_deg)
                self._last_remote_target_applied = list(self._bridge.latest_target_deg)
            self._last_live_sent_deg = list(self._target_deg)
            prefix = "实时发送" if live_mode else "已发送目标姿态"
            self.status_label.setText(f"{prefix}: {[round(value, 1) for value in self._target_deg]}")
            return
        self.status_label.setText(f"发送失败: {message}")

    def _hold_current_pose(self) -> None:
        if self._bridge.latest_actual_deg is None:
            self.status_label.setText("还没有收到机械臂姿态，无法保持当前姿态。")
            return

        self._set_target_pose(self._bridge.latest_actual_deg)
        self._send_target(live_mode=False)
        self.status_label.setText("已发送保持当前姿态命令。")

    def _apply_target_limits_if_available(self) -> None:
        if self._bridge.target_min_deg is None or self._bridge.target_max_deg is None:
            return

        if self._target_limits_applied:
            return

        for row, minimum, maximum in zip(
            self._rows, self._bridge.target_min_deg, self._bridge.target_max_deg
        ):
            row.set_target_range(minimum, maximum)

        self._target_limits_applied = True

    def _sync_remote_target_if_idle(self, remote_target_deg: Optional[List[float]]) -> None:
        if remote_target_deg is None:
            return

        if self._last_remote_target_applied == remote_target_deg:
            return

        has_unsent_local_edits = (
            (not self.live_apply_checkbox.isChecked())
            and self._last_live_sent_deg is not None
            and any(
                abs(current - sent) >= 0.05
                for current, sent in zip(self._target_deg, self._last_live_sent_deg)
            )
        )
        if has_unsent_local_edits:
            return

        if (time.monotonic() - self._last_local_target_edit_time) < 0.4:
            return

        if any(
            abs(current - remote) >= 0.05 for current, remote in zip(self._target_deg, remote_target_deg)
        ):
            self._set_target_pose(remote_target_deg)

        self._last_remote_target_applied = list(remote_target_deg)


def main() -> int:
    rclpy.init()
    bridge = ArmPoseDebugBridge()

    app = QApplication(sys.argv)
    app.setFont(QFont("Noto Sans CJK SC", 10))
    panel = ArmPoseDebugPanel(bridge)
    panel.show()

    exit_code = 0
    try:
        exit_code = app.exec_()
    finally:
        panel.close()
        bridge.destroy_node()
        if rclpy.ok():
            rclpy.shutdown()

    return int(exit_code)


if __name__ == "__main__":
    sys.exit(main())
