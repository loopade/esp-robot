from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument
from launch.substitutions import LaunchConfiguration, PathJoinSubstitution
from launch_ros.actions import Node
from launch_ros.substitutions import FindPackageShare


def generate_launch_description():
    # 此启动文件只启动确定性的 ROS 控制节点。
    config_file = LaunchConfiguration("config_file")
    use_sim_backend = LaunchConfiguration("use_sim_backend")

    default_config = PathJoinSubstitution(
        [FindPackageShare("arm_control"), "config", "servo_driver.yaml"]
    )

    return LaunchDescription(
        [
            DeclareLaunchArgument(
                "config_file",
                default_value=default_config,
                description="舵机驱动参数文件。",
            ),
            DeclareLaunchArgument(
                "use_sim_backend",
                default_value="true",
                description="使用仿真后端，不连接 PCA9685 硬件。",
            ),
            Node(
                package="arm_control",
                executable="servo_driver",
                name="servo_driver",
                output="screen",
                parameters=[config_file, {"use_sim_backend": use_sim_backend}],
            ),
        ]
    )
