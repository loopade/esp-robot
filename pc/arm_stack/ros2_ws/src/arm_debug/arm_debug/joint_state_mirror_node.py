import math
import time
from typing import Optional

import rclpy
from rclpy.node import Node
from sensor_msgs.msg import JointState


class JointStateMirrorNode(Node):
    def __init__(self) -> None:
        super().__init__("joint_state_mirror")

        self.declare_parameter("input_topic", "/debug_joint_states")
        self.declare_parameter("output_topic", "/arm/goal_joint_states")
        self.declare_parameter("deadband_deg", 0.25)
        self.declare_parameter("min_publish_interval_sec", 0.05)

        self.input_topic = self.get_parameter("input_topic").value
        self.output_topic = self.get_parameter("output_topic").value
        self.deadband_deg = float(self.get_parameter("deadband_deg").value)
        self.min_publish_interval_sec = float(
            self.get_parameter("min_publish_interval_sec").value
        )

        self.last_msg: Optional[JointState] = None
        self.last_publish_time = 0.0

        self.publisher = self.create_publisher(JointState, self.output_topic, 10)
        self.subscription = self.create_subscription(
            JointState, self.input_topic, self._on_joint_state, 10
        )

        self.get_logger().info(
            f"Joint state mirror ready. {self.input_topic} -> {self.output_topic}"
        )

    def _on_joint_state(self, msg: JointState) -> None:
        if not msg.position:
            return

        now = time.monotonic()
        if now - self.last_publish_time < self.min_publish_interval_sec:
            return

        if self.last_msg is not None and self._within_deadband(self.last_msg, msg):
            return

        outgoing = JointState()
        outgoing.header.stamp = self.get_clock().now().to_msg()
        outgoing.name = list(msg.name)
        outgoing.position = list(msg.position)
        self.publisher.publish(outgoing)

        self.last_msg = outgoing
        self.last_publish_time = now

    def _within_deadband(self, previous: JointState, current: JointState) -> bool:
        if previous.name != current.name or len(previous.position) != len(current.position):
            return False

        for previous_position, current_position in zip(previous.position, current.position):
            delta_deg = abs(math.degrees(current_position - previous_position))
            if delta_deg > self.deadband_deg:
                return False

        return True


def main(args=None) -> None:
    rclpy.init(args=args)
    node = JointStateMirrorNode()
    try:
        rclpy.spin(node)
    except KeyboardInterrupt:
        pass
    finally:
        node.destroy_node()
        rclpy.shutdown()

