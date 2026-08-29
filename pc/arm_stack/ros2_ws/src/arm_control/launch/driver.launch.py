from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument
from launch.conditions import IfCondition
from launch.substitutions import LaunchConfiguration, PathJoinSubstitution
from launch_ros.actions import Node
from launch_ros.substitutions import FindPackageShare


def generate_launch_description():
    config_file = LaunchConfiguration("config_file")
    use_sim_backend = LaunchConfiguration("use_sim_backend")
    enable_pose_http_bridge = LaunchConfiguration("enable_pose_http_bridge")

    default_config = PathJoinSubstitution(
        [FindPackageShare("arm_control"), "config", "servo_driver.yaml"]
    )

    return LaunchDescription(
        [
            DeclareLaunchArgument(
                "config_file",
                default_value=default_config,
                description="Servo driver parameter file.",
            ),
            DeclareLaunchArgument(
                "use_sim_backend",
                default_value="true",
                description="Use simulation backend instead of PCA9685 hardware.",
            ),
            DeclareLaunchArgument(
                "enable_pose_http_bridge",
                default_value="false",
                description="Start legacy HTTP bridge for non-ROS control clients.",
            ),
            Node(
                package="arm_control",
                executable="servo_driver",
                name="servo_driver",
                output="screen",
                parameters=[config_file, {"use_sim_backend": use_sim_backend}],
            ),
            Node(
                package="arm_control",
                executable="pose_http_bridge",
                name="pose_http_bridge",
                output="screen",
                parameters=[config_file],
                condition=IfCondition(enable_pose_http_bridge),
            ),
        ]
    )
