"""发布机械臂可动关节的零位状态，供 RViz 初始显示使用。"""

import rclpy
from rclpy.node import Node
from sensor_msgs.msg import JointState


class JointStateZeroPublisher(Node):
    """在没有真实控制器时持续发布零位关节角。"""

    def __init__(self):
        super().__init__('joint_state_zero_publisher')
        # 这些是 Fusion 导出的旋转关节；固定关节不需要发布状态。
        self.joint_names = [
            '旋转 2', '旋转 10', '旋转 35', '旋转 36',
            '旋转 37', '旋转 38', '旋转 39',
        ]
        self.publisher = self.create_publisher(JointState, '/joint_states', 10)
        self.timer = self.create_timer(0.1, self.publish_zero_state)

    def publish_zero_state(self):
        message = JointState()
        message.header.stamp = self.get_clock().now().to_msg()
        message.name = self.joint_names
        message.position = [0.0] * len(self.joint_names)
        self.publisher.publish(message)


def main(args=None):
    rclpy.init(args=args)
    node = JointStateZeroPublisher()
    try:
        rclpy.spin(node)
    finally:
        node.destroy_node()
        rclpy.shutdown()
