# arm-stack

当前仓库只保留三个业务目录：

- `pc`：Windows 侧入口，包含 Podman 启停脚本、Xbox 手柄脚本、ROS 2 工作区
- `esp32`：ESP32 + PCA9685 固件，负责接收上位机目标角度并输出 PWM
- `pi`：预留目录，当前不维护，也不参与运行

当前实际架构是 `PC + ESP32`：

1. ROS 2、RViz、调试面板运行在 PC 侧容器中
2. `servo_driver` 通过 UDP 将目标姿态发给 ESP32
3. ESP32 接收后驱动 PCA9685 控制机械臂舵机
4. `pc/xbox_control.ps1` 在 Windows 上运行，通过本地 HTTP bridge 复用同一套控制链路

## 目录

```text
pc/
  Containerfile
  podman_up.ps1
  podman_down.ps1
  run_panel.sh
  xbox_control.ps1
  ros2_ws/
arm/
  Cargo.toml
  Cargo.lock
  build.rs
  sdkconfig.defaults
  src/
pi/
```

## PC 侧启动

启动容器并拉起面板：

```powershell
cd <repo>\pc
.\podman_up.ps1 -Launch
```

只启动容器：

```powershell
cd <repo>\pc
.\podman_up.ps1
```

容器启动后，如需手动拉起面板：

```powershell
podman exec -it arm-pc-ros bash -lc "cd /workspace/arm-stack/pc && bash run_panel.sh"
```

停止容器：

```powershell
cd <repo>\pc
.\podman_down.ps1
```

同时停止 Windows 上的 Xbox 控制脚本：

```powershell
cd <repo>\pc
.\podman_down.ps1 -StopXbox
```

## Xbox 手柄

Windows 上直接运行：

```powershell
cd <repo>\pc
.\xbox_control.ps1
```

默认访问本机 `127.0.0.1:8765` 的 HTTP bridge，并复用面板控制同一条下发链路。

## ESP32

固件目录：

```powershell
cd <repo>\arm
cargo run
```

## GitHub 提交约定

建议只提交示例配置文件：

- `arm/.cargo/config.example.toml`
- `pc/ros2_ws/src/arm_control/config/servo_driver.example.yaml`
- `pc/ros2_ws/src/arm_control/config/servo_driver_esp32.example.yaml`

本地实际运行文件不会提交：

- `arm/.cargo/config.toml`
- `pc/ros2_ws/src/arm_control/config/servo_driver.yaml`
- `pc/ros2_ws/src/arm_control/config/servo_driver_esp32.yaml`

这些本地文件已经加入 `.gitignore`，用于保留 Wi-Fi、局域网地址和本机调试配置。
