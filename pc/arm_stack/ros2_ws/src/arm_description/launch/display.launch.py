from launch import LaunchDescription
from launch.substitutions import Command, PathJoinSubstitution
from launch_ros.actions import Node
from launch_ros.substitutions import FindPackageShare


def generate_launch_description():
    # 这里只发布模型；关节状态由 servo_driver 或测试命令发布。
    model_file = PathJoinSubstitution(
        [FindPackageShare("arm_description"), "urdf", "arm_platform.urdf.xacro"]
    )
    rviz_config = PathJoinSubstitution(
        [FindPackageShare("arm_description"), "rviz", "arm_debug.rviz"]
    )
    robot_description = Command(["xacro ", model_file])

    return LaunchDescription(
        [
            Node(
                package="robot_state_publisher",
                executable="robot_state_publisher",
                output="screen",
                parameters=[{"robot_description": robot_description}],
            ),
            Node(
                package="rviz2",
                executable="rviz2",
                output="screen",
                arguments=["-d", rviz_config],
                parameters=[{"robot_description": robot_description}],
            ),
        ]
    )
