"""一键启动模型、驱动、状态发布器和 RViz。"""

from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument
from launch.substitutions import Command, LaunchConfiguration, PathJoinSubstitution
from launch_ros.actions import Node
from launch_ros.substitutions import FindPackageShare


def generate_launch_description():
    # 通过一个配置参数同时支持仿真模式和 ESP32 UDP 模式。
    control_config = LaunchConfiguration("control_config")
    rviz_config = LaunchConfiguration("rviz_config")
    model_file = PathJoinSubstitution(
        [FindPackageShare("arm_description"), "urdf", "arm_platform.urdf.xacro"]
    )
    default_config = PathJoinSubstitution(
        [FindPackageShare("arm_control"), "config", "servo_driver.yaml"]
    )
    default_rviz = PathJoinSubstitution(
        [FindPackageShare("arm_description"), "rviz", "arm_debug.rviz"]
    )
    robot_description = Command(["xacro ", model_file])

    return LaunchDescription(
        [
            DeclareLaunchArgument("control_config", default_value=default_config),
            DeclareLaunchArgument("rviz_config", default_value=default_rviz),
            Node(
                package="arm_control",
                executable="servo_driver",
                name="servo_driver",
                output="screen",
                parameters=[control_config],
            ),
            Node(
                package="robot_state_publisher",
                executable="robot_state_publisher",
                name="robot_state_publisher",
                output="screen",
                parameters=[{"robot_description": robot_description}],
            ),
            Node(
                package="rviz2",
                executable="rviz2",
                name="rviz2",
                output="screen",
                arguments=["-d", rviz_config],
            ),
        ]
    )
