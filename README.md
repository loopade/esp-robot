# ESP Robot

仓库按设备职责分为三个目录：

```text
chassis/                 底盘 ESP32 固件
arm/                     机械臂 ESP32 固件和 STL 模型
pc/                      电脑端程序
  control_center/         底盘和机械臂统一控制/Web 通道（Rust）
  arm_stack/             机械臂 ROS 2、Podman 和 Xbox 控制
```

底盘固件：

```powershell
cd chassis
cargo build --release
```

机械臂固件：

```powershell
cd arm
cargo run --release --bin esp32-arm
```

机械臂接线（ESP32 与 PCA9685 共地）：

- ESP32 GPIO21 接 PCA9685 SDA，GPIO22 接 SCL；PCA9685 的 VCC 接 ESP32 逻辑电源。
- PCA9685 的舵机电源端接匹配舵机规格的独立电源，电源地与 ESP32/PCA9685 共地；不要从 ESP32 的 3.3V 引脚给舵机供电。
- 五个 180° 舵机依次插入 PCA9685 的 CH0–CH4；如需固定姿态输出，舵机插 CH12。按模块标注对齐信号、V+ 和 GND。

机械臂电脑端：

```powershell
cd pc\arm_stack
.\podman_up.ps1 -Launch
```

底盘电脑端：

```powershell
cd pc\control_center
cargo run --release -- web
```
