# -*- coding: utf-8 -*-
from launch_ros.actions import Node
from launch import LaunchDescription
from launch.actions import SetEnvironmentVariable
import xacro
import os
from ament_index_python.packages import get_package_share_directory


def generate_launch_description():
    share_dir = get_package_share_directory('arm_description')

    urdf_dir = os.path.join(share_dir, 'urdf')
    # 只选择包含 link 的主模型，不能误读 materials.xacro 等辅助宏。
    xacro_candidates = [
        os.path.join(urdf_dir, name) for name in os.listdir(urdf_dir)
        if name.endswith('.xacro') and name != 'materials.xacro'
    ]
    xacro_file = next(
        path for path in xacro_candidates
        if '<link ' in open(path, encoding='utf-8').read()
    )
    robot_description_config = xacro.process_file(xacro_file)
    robot_urdf = robot_description_config.toxml()

    rviz_config_file = os.path.join(share_dir, 'config', 'display.rviz')

    robot_state_publisher_node = Node(
        package='robot_state_publisher',
        executable='robot_state_publisher',
        name='robot_state_publisher',
        parameters=[
            {'robot_description': robot_urdf}
        ]
    )

    # 发布所有可动关节的零位状态，否则 robot_state_publisher 不会发布旋转关节 TF。
    joint_state_publisher_node = Node(
        package='joint_state_publisher',
        executable='joint_state_publisher',
        name='joint_state_publisher'
    )

    rviz_node = Node(
        package='rviz2',
        executable='rviz2',
        name='rviz2',
        arguments=['-d', rviz_config_file],
        output='screen'
    )

    return LaunchDescription([
        SetEnvironmentVariable('QT_QPA_PLATFORM', 'xcb'),
        SetEnvironmentVariable('QT_X11_NO_MITSHM', '1'),
        SetEnvironmentVariable('QT_XCB_GL_INTEGRATION', 'none'),
        SetEnvironmentVariable('LIBGL_ALWAYS_SOFTWARE', '1'),
        SetEnvironmentVariable('LIBGL_ALWAYS_INDIRECT', '0'),
        SetEnvironmentVariable('MESA_LOADER_DRIVER_OVERRIDE', 'llvmpipe'),
        robot_state_publisher_node,
        joint_state_publisher_node,
        rviz_node
    ])
