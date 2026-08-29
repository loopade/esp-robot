import math
import sys
from typing import List

import rclpy
from rclpy.node import Node
from sensor_msgs.msg import JointState


class JointCommandCli(Node):
    def __init__(self, joint_names: List[str], positions_deg: List[float]) -> None:
        super().__init__("joint_command_cli")
        self._publisher = self.create_publisher(JointState, "/arm/goal_joint_states", 10)
        self._joint_names = joint_names
        self._positions_deg = positions_deg
        self._timer = self.create_timer(0.5, self._publish_once)
        self._published = False

    def _publish_once(self) -> None:
        if self._published:
            self._timer.cancel()
            rclpy.shutdown()
            return

        msg = JointState()
        msg.header.stamp = self.get_clock().now().to_msg()
        msg.name = list(self._joint_names)
        msg.position = [math.radians(value) for value in self._positions_deg]
        self._publisher.publish(msg)
        self.get_logger().info(
            f"Published joint target (deg): {dict(zip(self._joint_names, self._positions_deg))}"
        )
        self._published = True


def main(args=None) -> None:
    import argparse

    parser = argparse.ArgumentParser(description="Publish one conservative joint target.")
    parser.add_argument("--joint", required=True, help="Joint name, for example joint_1")
    parser.add_argument("--deg", required=True, type=float, help="Target angle in degrees")
    parsed_args = parser.parse_args(args=args)

    joint_names = [f"joint_{index}" for index in range(1, 7)]
    positions_deg = [0.0] * 6

    if parsed_args.joint not in joint_names:
        raise SystemExit(f"Unknown joint: {parsed_args.joint}")

    positions_deg[joint_names.index(parsed_args.joint)] = parsed_args.deg

    rclpy.init()
    node = JointCommandCli(joint_names, positions_deg)
    try:
        rclpy.spin(node)
    except KeyboardInterrupt:
        pass
    finally:
        if hasattr(node, "context") and node.context.ok():
            node.destroy_node()
        if rclpy.ok():
            rclpy.shutdown()

    return 0


if __name__ == "__main__":
    sys.exit(main())
