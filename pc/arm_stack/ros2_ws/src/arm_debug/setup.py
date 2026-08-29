from setuptools import setup

package_name = "arm_debug"

setup(
    name=package_name,
    version="0.1.0",
    packages=[package_name],
    data_files=[
        ("share/ament_index/resource_index/packages", ["resource/arm_debug"]),
        ("share/arm_debug", ["package.xml"]),
        ("share/arm_debug/launch", ["launch/visual_debug.launch.py"]),
    ],
    install_requires=["setuptools"],
    zip_safe=True,
    maintainer="arm-stack",
    maintainer_email="dev@example.com",
    description="Visualization debug bridge for the ESP32 arm platform.",
    license="MIT",
    entry_points={
        "console_scripts": [
            "joint_state_mirror = arm_debug.joint_state_mirror_node:main",
            "joint_command_cli = arm_debug.joint_command_cli:main",
            "pose_command_cli = arm_debug.pose_command_cli:main",
            "pose_debug_panel = arm_debug.pose_debug_panel:main",
        ],
    },
)
