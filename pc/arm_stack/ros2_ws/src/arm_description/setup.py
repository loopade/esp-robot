from glob import glob

from setuptools import setup

package_name = "arm_description"

setup(
    name=package_name,
    version="0.1.0",
    packages=[package_name],
    data_files=[
        ("share/ament_index/resource_index/packages", ["resource/arm_description"]),
        ("share/arm_description", ["package.xml"]),
        ("share/arm_description/launch", ["launch/display.launch.py"]),
        ("share/arm_description/urdf", ["urdf/arm_platform.urdf.xacro"]),
        ("share/arm_description/rviz", ["rviz/arm_debug.rviz"]),
        ("share/arm_description/meshes", glob("meshes/*.stl")),
    ],
    install_requires=["setuptools"],
    zip_safe=True,
    maintainer="arm-stack",
    maintainer_email="dev@example.com",
    description="Robot description resources for the ESP32 arm platform.",
    license="MIT",
)
