# ESP Robot 数字孪生实施清单

本文档按“先在 WSL2 跑通仿真，再接入实体，最后开发网页三维界面”的顺序执行。

## 目标

完成以下闭环：

```text
Fusion 360 模型 -> ROS 2 / Xacro -> RViz
                         ^           |
                         |           v
ESP32 舵机状态 <- UDP <- ROS 2 控制节点 <- 目标关节角
```

第一阶段的验收标准：

- WSL2 可以编译 ROS 2 工作区。
- RViz 可以显示完整机械臂模型。
- 仿真关节命令可以驱动 RViz 模型。
- ESP32 可以通过 UDP 接收目标角度。
- 实体动作方向、零位和 RViz 一致。
- `/joint_states` 表示实体实际状态，或明确标记为估计状态。

## 0. 环境准备

## 0.1 Fusion 模型包路径约定

`arm_description` 不存放在主项目 Git 中。Fusion 360 导出器的保存目录必须选择：

```text
D:\OneDrive\Design\3D\机械臂\ros2
```

导出器会在该目录下生成：

```text
D:\OneDrive\Design\3D\机械臂\ros2\arm_description
```

WSL 工作区中的以下路径是指向上述目录的 Windows Junction：

```text
A:\Develop\esp-robot\pc\arm_stack\ros2_ws\src\arm_description
```

因此 ROS 2 仍然从工作区路径发现并构建 `arm_description`，但 Fusion 导出的 STL、Xacro、RViz 配置和启动文件不会进入主项目 Git。重新导出模型时直接覆盖 D 盘目录，不要在主项目下新建第二份 `arm_description`。

检查 Junction：

```powershell
Get-Item A:\Develop\esp-robot\pc\arm_stack\ros2_ws\src\arm_description
```

确认输出的 `Target` 为 `D:\OneDrive\Design\3D\机械臂\ros2\arm_description` 后，再在 WSL 中构建：

```bash
cd /mnt/a/Develop/esp-robot/pc/arm_stack/ros2_ws
source /opt/ros/lyrical/setup.bash
colcon build --symlink-install --packages-select arm_description
source install/setup.bash
```

建议在 WSL2 的 Linux 文件系统中工作，而不是直接在 `/mnt/a` 下编译：

```bash
cp -r /mnt/a/Develop/esp-robot ~/esp-robot
cd ~/esp-robot
```

确认 ROS 2 版本并加载环境：

```bash
source /opt/ros/<发行版>/setup.bash
printenv ROS_DISTRO
```

WSLg 可用时，RViz2 可以直接显示。如果没有图形界面，需要配置 WSLg 或使用远程 X11。

## 1. 编译 ROS 2 工作区

```bash
cd ~/esp-robot/pc/arm_stack/ros2_ws
source /opt/ros/<发行版>/setup.bash
rosdep update
rosdep install --from-paths src --ignore-src -r -y
colcon build --symlink-install
source install/setup.bash
```

每次重新打开终端后都要执行：

```bash
source /opt/ros/<发行版>/setup.bash
source ~/esp-robot/pc/arm_stack/ros2_ws/install/setup.bash
```

验收：`colcon build` 无错误，且以下命令能找到包：

```bash
ros2 pkg prefix arm_description
ros2 pkg prefix arm_control
```

## 2. 只显示 Fusion 360 模型

终端 A：

```bash
cd ~/esp-robot/pc/arm_stack/ros2_ws
source /opt/ros/<发行版>/setup.bash
source install/setup.bash
ros2 launch arm_description display.launch.py
```

验收：RViz 中能看到完整机械臂，固定坐标系为 `base_link`，没有 mesh 找不到或 Xacro 解析错误。

如果模型尺寸不对，先检查 `arm_platform.urdf.xacro` 中的：

- `mesh_scale`，当前假定 STL 单位为毫米；
- 各 link 的 `visual origin`；
- 各 joint 的 `origin` 和 `axis`。

## 3. 运行仿真控制闭环

终端 B：

```bash
source /opt/ros/<发行版>/setup.bash
source ~/esp-robot/pc/arm_stack/ros2_ws/install/setup.bash
ros2 launch arm_control driver.launch.py use_sim_backend:=true
```

终端 C：

```bash
source /opt/ros/<发行版>/setup.bash
source ~/esp-robot/pc/arm_stack/ros2_ws/install/setup.bash
ros2 launch arm_description display.launch.py
```

查看话题：

```bash
ros2 topic list
ros2 topic echo /joint_states
ros2 topic echo /arm/target_joint_states
```

发送测试命令：

```bash
ros2 topic pub --once /arm/goal_joint_states sensor_msgs/msg/JointState \
  "{name: [joint_1, joint_2, joint_3, joint_4, joint_5], position: [0.2, 0.3, -0.2, 0.1, 0.0]}"
```

验收：RViz 模型随目标关节角运动，`/joint_states` 持续发布，关节名称与 Xacro 完全一致。

## 4. 校准模型和实体关节

### 已确认的 Fusion 机械关系

以下关系已经根据当前 Fusion 360 装配确认，后续 ROS 关节映射必须以此为依据：

- `旋转 2`：MG996R 输出轴与 J1 主动齿轮之间的转动关节，是底座驱动链的主动输入。
- `旋转 10`：J1 中心轴与从动齿轮之间的转动关节，由 `旋转 2` 啮合带动，不是独立舵机输入；传动方向相反，角度比例为 `1:1`。
- `旋转 35`：大臂抬起关节。

`旋转 2` 到 `旋转 10` 应在 URDF/Xacro 中表达为反向 `1:1` 的齿轮联动，并在执行器映射中只把 `旋转 2` 作为独立控制输入。

在接电机之前，固定一个实体机械臂零位，并记录照片或角度。逐关节只测试 ±10°：

1. 确认 `joint_1` 到 `joint_5` 的对应关系。
2. 确认每个 joint 的旋转轴方向。
3. 确认 ROS 正方向和实体正方向一致。
4. 设置真实零位和角度偏移。
5. 设置机械软限位。

配置文件位于：

```text
pc/arm_stack/ros2_ws/src/arm_control/config/servo_driver_esp32.yaml
```

重点检查：`direction_signs`、`angle_offsets_deg`、`min_angle_deg`、`max_angle_deg`。

## 5. 接入 ESP32 UDP

复制或编辑 ESP32 配置：

```bash
cd ~/esp-robot/pc/arm_stack/ros2_ws/src/arm_control/config
cp servo_driver_esp32.example.yaml servo_driver_esp32.yaml
```

修改至少这些参数：

```yaml
use_sim_backend: false
backend_mode: "esp32_udp"
esp32_udp_host: "<ESP32 的局域网 IP>"
```

启动：

```bash
source /opt/ros/<发行版>/setup.bash
source ~/esp-robot/pc/arm_stack/ros2_ws/install/setup.bash
ros2 launch arm_debug visual_debug.launch.py \
  control_config:=<安装后的配置文件路径>
```

先用小角度测试，确认急停和断网时不会继续运动。验收：

- ESP32 能收到目标角度；
- 实体运动方向与 RViz 一致；
- 断开 ROS 2 或网络后，舵机进入安全状态；
- `/joint_states` 的来源和含义已经确认。

如果 ESP32 没有位置传感器，当前 `/joint_states` 可能只是目标或估计状态。真正的数字孪生需要后续增加实际位置、在线状态、故障码、电压、电流和温度反馈。

## 6. 固定 ROS 2 数字孪生接口

建议长期保留以下接口：

```text
/arm/goal_joint_states       控制目标
/arm/target_joint_states     当前规划目标
/joint_states                实际或估计关节状态
/tf                          机器人坐标变换
/arm/status                  在线、故障、急停状态
/arm/telemetry               电压、电流、温度、网络延迟
```

现有代码已经覆盖前三类的主要部分。实体闭环稳定后，再增加 `/arm/status` 和 `/arm/telemetry`。

## 7. 开发网页三维界面

不要在 RViz 闭环稳定前开始网页端。网页端第一版只做：

1. 加载与 ROS 相同的 URDF/STL 模型。
2. 订阅 `/joint_states` 和 `/tf`。
3. 实时显示实体姿态。
4. 显示五个关节角、连接状态和故障状态。
5. 支持安全范围内的单关节测试命令。

推荐架构：

```text
ROS 2 <-> rosbridge/WebSocket <-> Three.js 网页
```

网页端使用实际状态驱动模型，目标状态只作为辅助显示，避免模型看起来已经运动但实体没有运动。

## 8. 提交前检查

仓库要求：烧录成功后才能提交代码。提交前执行：

- ROS 2 工作区重新 `colcon build`；
- 仿真闭环通过；
- ESP32 固件成功烧录；
- 实体小角度动作测试通过；
- 网络断开和进程退出时舵机安全停机；
- `git status` 中确认模型文件删除或迁移是有意的。

完成以上检查后再提交 Git。不要把未验证的 ESP32 IP、临时网络配置或本地路径提交到公共配置中。

