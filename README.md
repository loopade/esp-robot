# ESP Robot

仓库包含 ESP32 固件、电脑端控制程序和机械臂模型：

```text
chassis/                 底盘与机械臂合并固件
arm/                     机械臂 STL 模型及旧固件源码
pc/
  control_center/         底盘和机械臂统一控制/Web 控制台（Rust）
  arm_stack/              机械臂 ROS 2、Podman 和 Xbox 控制
```

## ESP32 固件

固件统一从 `chassis/` 构建，默认同时启用底盘和机械臂。两个 feature 可分别启用：

```powershell
cd chassis
cargo +esp build --release --target xtensa-esp32-espidf
cargo +esp build --release --target xtensa-esp32-espidf --no-default-features --features chassis
cargo +esp build --release --target xtensa-esp32-espidf --no-default-features --features arm
```

Wi-Fi 是公共功能，两个模块共用一次连接。ESP-IDF 工具链和本机 Wi-Fi 配置按 `chassis/.cargo/config.toml` 设置。

机械臂接线（ESP32 与 PCA9685 共地）：

- ESP32 GPIO21 接 PCA9685 SDA，GPIO22 接 SCL；PCA9685 的 VCC 接 ESP32 逻辑电源。
- PCA9685 舵机电源端使用匹配舵机规格的独立电源，并与 ESP32/PCA9685 共地；不要从 ESP32 的 3.3V 引脚给舵机供电。
- 五个 180° 舵机依次接 PCA9685 的 CH0–CH4；固定姿态舵机接 CH12。按模块标注对齐信号、V+ 和 GND。

## 电脑端控制台

启动底盘和机械臂统一控制台（Web）：

```powershell
cd pc\control_center
cargo run --release -- web
```

机械臂 ROS 2 / Xbox 控制环境：

```powershell
cd pc\arm_stack
.\podman_up.ps1 -Launch
```
Xbox 手柄控制：在 Windows 连接手柄后，另开一个 PowerShell 运行：

```powershell
cd pc\arm_stack
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\xbox_control.ps1
```

按键：左摇杆上下/左右控制底盘前进和转向；方向键左右控制机械臂底座，方向键上下控制肩关节；右摇杆控制肘关节和腕俯仰；LB/RB 控制腕旋转，LT/RT 控制夹爪。X 切换精细模式，Y 重置速度。松开左摇杆即停车，Ctrl+C 退出时会发送停车指令。