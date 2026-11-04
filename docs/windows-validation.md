# Windows 烧录、录音与验收

## ST-LINK 固件升级（本板已完成）

本板已使用 STSW-LINK007 3.18.12 的 Java 命令行工具完成 V2J14S0 → 2.49.0 升级。
下列旧固件错误及恢复流程保留用于复现和其他板子；不再是当前阻碍。本板重新插拔 CN5 后已成功枚举，并通过 Windows 10 上的 10 秒录音和复位后独立运行的 3 秒录音验证。

2026-11-05 检测到 `0483:3748` STLink V2；`probe-rs 0.32.0 info` 返回：

```text
The firmware on the probe is outdated, and not supported by probe-rs.
The minimum supported firmware version is 26.
Use the ST-Link updater utility to update your probe firmware.
```

这不是目标应用编译错误。该记录来自升级前；升级、烧录和当前 USB 状态见 [验证报告](validation.md)。

1. 从 [STSW-LINK007 官方页面](https://www.st.com/en/development-tools/stsw-link007.html) 获取 Windows ST-LINK 升级工具，或使用已安装的 STM32CubeProgrammer 的 ST-LINK 升级功能。
2. CN1 保持供电，关闭占用 ST-LINK 的调试器，按官方工具提示连接、查看并升级 ST-LINK/V2 固件。
3. 升级结束按提示重新插拔 CN1，重新运行 `probe-rs list` 与 `probe-rs info`。
4. 确认目标属于 STM32F407、板内 CN2 SWD 跳帽齐全，再烧录。未验证目标身份时不要使用整片擦除或修改 option bytes。

工具安装可使用 [probe-rs 官方安装说明](https://probe.rs/docs/getting-started/installation/)，例如 `cargo install probe-rs-tools --locked`。
本轮使用下载后通过官方 `.sha256` 校验的 probe-rs 0.32.0 Windows 预编译程序，保存在工作区 `work/tools`，未修改系统 PATH。
ST-LINK 在 Windows 使用官方 ST 驱动；不要为 CN5 USB Audio 接口替换成 WinUSB/Zadig 驱动。

有 Java 运行时的环境，可在升级包的 `AllPlatforms` 目录用官方命令行接口：

```powershell
java -jar STLinkUpgrade.jar -list
java -jar STLinkUpgrade.jar -checkVer
java -jar STLinkUpgrade.jar -update
```

这些命令已对本板实际执行成功。`-update` 为正常升级；不需要 `-force_prog` 或指定另一种探针功能类型。
`probe-rs info` 对本板不能自动识别型号，并且会忽略 `--chip`；确认 SWD 可用应使用显式目标的 read/run。
Cargo runner 已加入 `--speed 1000 --connect-under-reset --verify --rtt-channel-mode no-block-skip`；探针实际协商为 950 kHz。

## 接线和启动

- 两根数据线：CN1（Mini-B）是 ST-LINK/电源；CN5（Micro-AB）是应用 USB OTG FS。
- 保留 CN2 两个跳帽；BOOT0 保持正常 Flash 启动；确认 JP1/板上电源跳帽原状。
- 默认模式输出真实麦克风；`--no-default-features` 才输出阶段一的软件正弦波。实体麦克风录音和稳定性结果见 [阶段四报告](stage4-live.md)。

```powershell
cargo run --locked --release
```

应依次观察 RTT：`usb-mic boot` → `clocks ready` → `descriptors ready` → `usb reset` → `usb configured=true`。
`stats vbus=true` 表示 PA9 数字输入读到高电平。若始终为 false，先重新插拔 CN5，检查其 VBUS、数据线和 USB 端口，不要先修改音频类描述符或关闭 VBUS 检测掩盖问题。
绿色 PD12 LED 每秒翻转。开始录音后应出现 `usb interface=1 alt=1`，累计 `packets` 每秒大约增加 1000。
停止录音可进入 alt=0 或 suspend；重新打开录音后应继续发送。

## 枚举和描述符验收

[Microsoft 的 UAC1 驱动说明](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/usb-audio-class-system-driver--usbaudio-sys-)介绍了系统 `Usbaudio.sys` 驱动。
此固件的预期为即插即用，不需要自定义音频驱动。

1. 设备管理器查看 `STM32F4 USB Microphone`，记录设备状态和驱动详情，目标为系统 `usbaudio.sys`。
2. 运行以下命令保存设备信息：

```powershell
Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match 'VID_C0DE&PID_CAFE' -or $_.FriendlyName -match 'STM32F4' } | Format-List Status,Class,FriendlyName,InstanceId
```

3. 使用 Windows SDK 的 USBView 检查并保存完整描述符：

| 项目 | 预期 |
| --- | --- |
| VID/PID | C0DE/CAFE，仅实验标识 |
| USB | 2.0 / Full Speed，EP0 64 bytes |
| AudioControl | class 01、subclass 01、protocol 00，bcdADC 0100 |
| 拓扑 | Input Terminal Microphone 0201 → Feature Unit 主通道 Mute → Output USB Streaming 0101 |
| AudioStreaming alt 0 | 零端点，空闲 |
| AudioStreaming alt 1 | 一条 Isochronous IN 端点 |
| Format Type I | PCM 0001；1 channel；2-byte subframe；16 bits；仅 16000 Hz（80 3E 00） |
| Endpoint | IN；bmAttributes 05（Isochronous / Asynchronous / Data）；bInterval 1；wMaxPacketSize 34 |
| 正常音频 payload | 每帧 32 bytes = 16 samples |

`wMaxPacketSize=34` 是容量，不能误判正常 32-byte 包为欠载；上游保留一个采样的余量。
Windows 录音 API 可能重采样，WAV 文件标注 16 kHz 本身不证明描述符正确。

## 录音和信号验收

Windows 设置 → 隐私/隐私和安全性 → 麦克风：允许桌面应用访问。
在声音输入或经典 `mmsys.cpl` 的录制设备中选择本设备，尽可能选择 1 channel、16-bit、16000 Hz；关闭增强、降噪、AGC、通信衰减和监听。
本固件未暴露音量控制，只暴露主通道 Mute；不要以 Windows 是否出现音量滑块判断枚举成败。

```powershell
python tools/record_windows.py --list
python tools/record_windows.py --device STM32F4 --seconds 10 --output captures/stage1.wav
python tools/analyze_tone.py captures/stage1.wav --json captures/stage1-analysis.json
```

可另用 Windows 录音机验证能打开设备和播放，定量检查优先使用脚本产生的无压缩 WAV。
推荐验收值（未处理录音）：1000 Hz 主峰，峰值约 8192（-12.04 dBFS），RMS 约 5793（-15.05 dBFS），DC 接近 0，无削顶，无整毫秒静音。
脚本允许一定幅度误差，输出 THD、16-sample 周期差异、静音段和 exit code；必须保留原 WAV 才能复核。
脚本不能证明无 USB 丢帧：正好丢失一个完整正弦周期仍可能看似连续，需配合 RTT/USB 抓包判断。

## 稳定性方法（本轮未执行）

1. 连续录音 10 分钟：`--seconds 600`。对比录音时长、实际耗时、波形和 RTT 计数，USB 错误在稳定运行中应不增加。
2. 录音启动/停止 20 次；CN1 保持供电时 CN5 插拔 20 次；每次 Windows 都应能重新打开录音。
3. 再测试 Windows 睡眠/恢复、USB 端口或 Hub 切换。逐项记录操作、时刻、设备状态和故障，不把一次成功称为长期稳定。
4. 保留开启和结束时的 RTT；恢复后 packets 必须继续增加，LED 不能停止。

统计解释：`packets` 是驱动接受的写包数，不是主机确认收到的包数（等时传输不重试）。`usb_errors` 是 API 暴露的 BufferOverflow；`disabled` 是端点失效次数，可由正常停止/拔线引起。
`discarded_samples` 只统计写包返回错误而丢弃的 16 个软件采样，不是端到端丢样总数。
`pending/high` 是软件当前/最高待发送采样数（0/16），最低水位固定为 0；并非尚未存在的 DMA 环形缓冲区。
`dma_active=false` 表示 DMA 错误、过载/欠载统计此时不适用。第二阶段必须加入真实 DMA 错误和环形缓冲区统计，第四阶段加入生产/消费丢样与时钟漂移策略。
计数器为 u32 累计并会回绕，固件重启后清零；不要把重启造成的计数下降解释为恢复正常。

## 出问题时的观察顺序与回传资料

| 现象 | 具体观察 |
| --- | --- |
| probe-rs 拒绝连接 | 保存 `probe-rs --version`、`probe-rs list`、`probe-rs info` 完整输出及 ST-LINK 升级工具版本截图 |
| 有 boot、没有 clocks ready | MCU 停在 HSE/PLL 初始化；核对 X2、PH0/PH1、板号及是否改过焊桥；不要盲改成其他板的时钟 |
| clocks ready 后 Windows 无设备 | 核对 CN5 数据线、设备管理器新增项/错误码、PA9 VBUS；测 CN5 VBUS 对 GND，确认 PC0 高；保存 RTT |
| Code 43/描述符失败 | 保存 USBView 输出、Windows 设备状态、reset/configured 日志；先解决 EP0/时钟问题 |
| 能枚举但录不到 | 检查隐私权限、所选设备、alt=1 是否到来、packets 是否增长、mute 是否开启 |
| 杂音/间断/重连后不恢复 | 保存 WAV、capture JSON、analysis JSON、RTT 前后计数、Windows 版本和发生故障的精确操作 |

如需示波器检查，仅在排错阶段由用户操作：8 MHz 晶振应使用适合的高阻探头以免扰动；本阶段 PB10 无 PDM 时钟属于预期。无需外接模块完成正常使用。
