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
cargo run
```

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
