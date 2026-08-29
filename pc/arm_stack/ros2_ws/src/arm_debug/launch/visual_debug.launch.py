from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument
from launch.conditions import IfCondition
from launch.substitutions import Command, LaunchConfiguration, PathJoinSubstitution
from launch_ros.actions import Node
from launch_ros.substitutions import FindPackageShare


def generate_launch_description():
    control_config_file = LaunchConfiguration("control_config_file")
    start_servo_driver = LaunchConfiguration("start_servo_driver")
    start_pose_http_bridge = LaunchConfiguration("start_pose_http_bridge")
    pose_http_listen_host = LaunchConfiguration("pose_http_listen_host")
    model_file = PathJoinSubstitution(
        [FindPackageShare("arm_description"), "urdf", "arm_platform.urdf.xacro"]
    )
    rviz_config = PathJoinSubstitution(
        [FindPackageShare("arm_description"), "rviz", "arm_debug.rviz"]
    )
    default_control_config = PathJoinSubstitution(
        [FindPackageShare("arm_control"), "config", "servo_driver_esp32.yaml"]
    )

    robot_description = Command(["xacro ", model_file])

    return LaunchDescription(
        [
            DeclareLaunchArgument(
                "control_config_file",
                default_value=default_control_config,
                description="Servo driver and panel parameter file.",
            ),
            DeclareLaunchArgument(
                "start_servo_driver",
                default_value="true",
                description="Start servo_driver on the PC side.",
            ),
            DeclareLaunchArgument(
                "start_pose_http_bridge",
                default_value="true",
                description="Start local HTTP bridge for the legacy Xbox control script.",
            ),
            DeclareLaunchArgument(
                "pose_http_listen_host",
                default_value="0.0.0.0",
                description="Listen host for the legacy HTTP bridge.",
            ),
            DeclareLaunchArgument(
                "rviz_config",
                default_value=rviz_config,
                description="RViz configuration file.",
            ),
            Node(
                package="arm_control",
                executable="servo_driver",
                name="servo_driver",
                output="screen",
                parameters=[control_config_file],
                condition=IfCondition(start_servo_driver),
            ),
            Node(
                package="arm_control",
                executable="pose_http_bridge",
                name="pose_http_bridge",
                output="screen",
                parameters=[control_config_file, {"listen_host": pose_http_listen_host}],
                condition=IfCondition(start_pose_http_bridge),
            ),
            Node(
                package="robot_state_publisher",
                executable="robot_state_publisher",
                name="robot_state_publisher",
                output="screen",
                parameters=[{"robot_description": robot_description}],
                remappings=[("/joint_states", "/debug_joint_states")],
            ),
            Node(
                package="arm_debug",
                executable="pose_debug_panel",
                name="pose_debug_panel",
                output="screen",
                parameters=[control_config_file],
            ),
            Node(
                package="rviz2",
                executable="rviz2",
                name="rviz2",
                output="screen",
                arguments=["-d", LaunchConfiguration("rviz_config")],
                parameters=[{"robot_description": robot_description}],
            ),
        ]
    )
