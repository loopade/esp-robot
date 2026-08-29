import math
import sys
from typing import List

import rclpy
from rclpy.node import Node
from sensor_msgs.msg import JointState


class PoseCommandCli(Node):
    def __init__(self, positions_deg: List[float]) -> None:
        super().__init__("pose_command_cli")
        self._publisher = self.create_publisher(JointState, "/arm/goal_joint_states", 10)
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
        msg.name = [f"joint_{index}" for index in range(1, 7)]
        msg.position = [math.radians(value) for value in self._positions_deg]
        self._publisher.publish(msg)
        self.get_logger().info(
            f"Published arm pose (deg): {dict(zip(msg.name, self._positions_deg))}"
        )
        self._published = True


def _parse_pose_deg(raw_pose: str) -> List[float]:
    parts = [item.strip() for item in raw_pose.split(",")]
    if len(parts) != 6:
        raise SystemExit("Pose must contain exactly 6 comma-separated degree values.")

    try:
        return [float(item) for item in parts]
    except ValueError as exc:
        raise SystemExit(f"Invalid pose value: {exc}") from exc


def main(args=None) -> int:
    import argparse

    parser = argparse.ArgumentParser(
        description="Publish one conservative full-arm target pose."
    )
    parser.add_argument(
        "--pose-deg",
        required=True,
        help="Six comma-separated joint targets in degrees, for example 0,10,-15,0,0,20",
    )
    parsed_args = parser.parse_args(args=args)

    positions_deg = _parse_pose_deg(parsed_args.pose_deg)

    rclpy.init()
    node = PoseCommandCli(positions_deg)
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
