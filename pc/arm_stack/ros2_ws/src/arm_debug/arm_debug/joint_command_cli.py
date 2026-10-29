"""用于安全测试的一次性关节角命令发布器。"""

import argparse
import math

import rclpy
from rclpy.node import Node
from sensor_msgs.msg import JointState

JOINT_NAMES = ["joint_1", "joint_2", "joint_3", "joint_4", "joint_5"]


class JointCommandPublisher(Node):
    """发布一条带关节名称的命令，然后退出。"""

    def __init__(self, degrees: list[float]) -> None:
        super().__init__("joint_command_cli")
        self.publisher = self.create_publisher(JointState, "/arm/goal_joint_states", 10)
        self.degrees = degrees
        self.sent = False
        self.timer = self.create_timer(0.2, self._publish_once)

    def _publish_once(self) -> None:
        if self.sent:
            return
        # JointState 使用弧度；命令行使用度数更适合人工测试。
        message = JointState()
        message.header.stamp = self.get_clock().now().to_msg()
        message.name = list(JOINT_NAMES)
        message.position = [math.radians(value) for value in self.degrees]
        self.publisher.publish(message)
        self.get_logger().info(f"已发布目标角度（度）：{self.degrees}")
        self.sent = True
        rclpy.shutdown()


def main(args=None) -> None:
    parser = argparse.ArgumentParser(description="发布五个机械臂关节角度，单位为度。")
    parser.add_argument("degrees", nargs=5, type=float, metavar="DEG")
    parsed, ros_args = parser.parse_known_args(args)
    rclpy.init(args=ros_args)
    node = JointCommandPublisher(parsed.degrees)
    rclpy.spin(node)
    node.destroy_node()


if __name__ == "__main__":
    main()
