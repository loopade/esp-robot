# ESP32 履带底盘（Rust + Wi-Fi 控制）

工程路径为 `A:\tank`。ESP32 通过 Wi-Fi 接收电脑/Web 控制、数字孪生、图像识别和自动驾驶指令；固件不启用蓝牙。

## 接线

接线时断开 18 V 电池。电机电流不能经过 ESP32、面包板或杜邦线。

| ESP32 | 左 BTS7960 / IBT-2 |
|---|---|
| GPIO25 | RPWM |
| GPIO26 | LPWM |
| GPIO27 | R_EN 与 L_EN（在驱动器端短接） |
| 5V/VIN | VCC |
| GND | GND |

| ESP32 | 右 BTS7960 / IBT-2 |
|---|---|
| GPIO32 | RPWM |
| GPIO33 | LPWM |
| GPIO14 | R_EN 与 L_EN（在驱动器端短接） |
| 5V/VIN | VCC |
| GND | GND |

每块 BTS7960：

- `M+`、`M-` 接该侧 18 V 380 电机。
- `B+` 经独立保险丝接 18 V 电池正极，`B-` 接电池负极。
- 两块驱动器 GND、ESP32 GND、降压模块 GND 必须共地。
- `R_IS`、`L_IS` 目前悬空，后续可接 ADC 做电流监测。
- 每块驱动器的 `B+`/`B-` 附近建议并联 1000-2200 uF、耐压至少 25 V 的低 ESR 电容。

用 18 V 转稳定 5 V、至少 1 A 的降压模块给 ESP32 `5V/VIN` 供电，绝不能把 5 V 接到
`3V3`。USB 烧录时建议先断开外部 5 V，除非已确认开发板具有电源路径隔离。

## 控制方式

- Web 页面或 UDP 指令控制左右履带速度与方向。
- 电脑指令 300 ms 无新数据时自动停车。
- 停车、掉线或指令超时时，程序同时将 PWM 清零并拉低左右两个 `EN` GPIO。
- 上电测试模式默认运行 5 秒、停止 5 秒并循环；收到第一条有效电脑指令后退出测试模式。

履带底盘只有两条平行履带，不能像麦克纳姆轮一样横向平移；“左右移动”实现为弧线转向或
原地差速旋转。

## Wi-Fi / 电脑扩展通道

修改 `.cargo/config.toml` 中的 `WIFI_SSID`、`WIFI_PASS`。

- ESP32 加入标准 IPv4 多播组 `239.255.42.99`。
- 多播 UDP `239.255.42.99:4210`：二进制自动驾驶指令。
- 多播 UDP `239.255.42.99:4211`：10 Hz JSON 遥测和设备自动发现。
- PC 无需配置或预先知道 ESP32 IP；从遥测数据包的来源地址自动显示当前设备地址。
- 自动指令格式：`AUT1` + `u16 LE sequence` + `i16 LE left` + `i16 LE right` + `u8 armed`。
- 左右值范围 `-1000..1000`。

Rust 示例：

```powershell
cd A:\tank\pc_channel
cargo run --release -- monitor

# 以 20% 速度直行 2 秒（架空履带测试）
cargo run --release -- drive 200 200 2
```

### Web 控制台

```powershell
cd A:\tank\pc_channel
cargo run --release -- web
```

然后打开 `http://127.0.0.1:8080`。页面提供按住运行的前进、后退、原地左右转、速度设置、急停和实时遥测；也可使用 `W/A/S/D`，空格键急停。浏览器停止刷新控制命令超过 350 ms 后，后端会自动发送停车指令。

图像识别程序只需按 50 Hz 向 `239.255.42.99:4210` 发送同一 `AUT1` 数据包；数字孪生加入
`239.255.42.99` 多播组后监听 4211 的 JSON。
后续编码器、IMU、里程计字段可追加到 JSON，不破坏现有控制协议。

## 构建与烧录

```powershell
cd A:\tank
cargo build --release
cargo run --release
```

若有多个串口：

```powershell
espflash flash --monitor --port COM5 target/xtensa-esp32-espidf/release/tracked-chassis
```

???????? `COM3`?????????? Wi-Fi ???

## 首次动力测试

先架空两条履带并降低保险丝/电源限流，通过Web控制台低速测试。若某侧方向相反，断电后交换该电机
`M+`/`M-`，或将 `src/main.rs` 对应的 `LEFT_POLARITY`/`RIGHT_POLARITY` 改为 `-1`。
确认松开页面按钮、关闭控制页面、网络断开和指令超时都能停车后，再落地测试。
