#!/usr/bin/env bash
set -euo pipefail

ROS_DISTRO_NAME="${ROS_DISTRO_NAME:-jazzy}"

set +u
source "/opt/ros/${ROS_DISTRO_NAME}/setup.bash"
set -u

cd /workspace/arm-stack/pc/arm_stack/ros2_ws
colcon build --symlink-install

set +u
source install/setup.bash
set -u

ros2 launch arm_debug visual_debug.launch.py
