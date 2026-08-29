from setuptools import setup

package_name = "arm_control"

setup(
    name=package_name,
    version="0.1.0",
    packages=[package_name],
    data_files=[
        ("share/ament_index/resource_index/packages", ["resource/arm_control"]),
        ("share/arm_control", ["package.xml"]),
        ("share/arm_control/launch", ["launch/driver.launch.py"]),
        (
            "share/arm_control/config",
            ["config/servo_driver.yaml", "config/servo_driver_esp32.yaml"],
        ),
    ],
    install_requires=["setuptools"],
    zip_safe=True,
    maintainer="arm-stack",
    maintainer_email="dev@example.com",
    description="Servo driver and smooth motion control for the ESP32 arm platform.",
    license="MIT",
    entry_points={
        "console_scripts": [
            "servo_driver = arm_control.servo_driver_node:main",
            "pose_http_bridge = arm_control.pose_http_bridge_node:main",
        ],
    },
)
