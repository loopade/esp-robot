from launch import LaunchDescription
from launch.substitutions import Command, PathJoinSubstitution
from launch_ros.actions import Node
from launch_ros.substitutions import FindPackageShare


def generate_launch_description():
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
                package="joint_state_publisher_gui",
                executable="joint_state_publisher_gui",
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
