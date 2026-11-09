# Windows 烧录、录音与验收

适用于当前默认 `live-mic` + `noise-suppression` 固件。启动后保持安静直到校准完成；通常至少约 5 秒，活动会延长等待，详见 [校准保护](calibration-guard.md)。现场校准、原声切换和新映像验证见 [板上降噪](live-denoise.md)。接线与模式速查见 [README](../README.md)，计数字段见 [架构说明](architecture.md)。
截至 2026-11-05，已保存 Windows 10 Enterprise 19045 上 600 秒连续实体麦克风录音、三次顺序重开和复位后无调试器的 10 秒录音记录，见 [阶段四报告](stage4-live.md)。Windows 11 尚未实测；下列操作是复验指南，不表示本次文档更新重新进行了硬件测试。

## ST-LINK 固件升级（本板已完成）

本板已使用 STSW-LINK007 3.18.12 的 Java 命令行工具完成 V2J14S0 → 2.49.0 升级。
下列旧固件错误及恢复流程保留用于复现和其他板子；不再是当前阻碍。升级和初次正弦录音记录见 [第一阶段报告](validation.md)，当前真实麦克风结果见阶段四报告。

2026-11-05 检测到 `0483:3748` STLink V2；`probe-rs 0.32.0 info` 返回：

```text
The firmware on the probe is outdated, and not supported by probe-rs.
The minimum supported firmware version is 26.
Use the ST-Link updater utility to update your probe firmware.
```

这不是目标应用编译错误。该记录来自升级前；升级、烧录和当前 USB 状态见 [验证报告](validation.md)。

1. 从 [STSW-LINK007 官方页面](https://www.st.com/en/development-tools/stsw-link007.html) 获取 Windows ST-LINK 升级工具，或使用已安装的 STM32CubeProgrammer 的 ST-LINK 升级功能。
2. CN1 保持供电，关闭占用 ST-LINK 的调试器，按官方工具提示连接、查看并升级 ST-LINK/V2 固件。
3. 升级结束按提示重新插拔 CN1，重新运行 `probe-rs list`。可保存 `probe-rs info` 的自动识别输出；本板须结合下述显式目标操作判断 SWD 是否可用。
4. 确认目标属于 STM32F407、板内 CN2 SWD 跳帽齐全，再烧录。未验证目标身份时不要使用整片擦除或修改 option bytes。

工具安装可使用 [probe-rs 官方安装说明](https://probe.rs/docs/getting-started/installation/)，例如 `cargo install probe-rs-tools --locked`。
历史验收使用通过官方 `.sha256` 校验的 probe-rs 0.32.0 Windows 预编译程序；当时工具路径为 `work/tools`，该本地目录不随仓库交付。新环境需自行安装并加入 PATH。
ST-LINK 在 Windows 使用官方 ST 驱动；不要为 CN5 USB Audio 接口替换成 WinUSB/Zadig 驱动。

有 Java 运行时的环境，可在升级包的 `AllPlatforms` 目录用官方命令行接口：

```powershell
java -jar STLinkUpgrade.jar -list
java -jar STLinkUpgrade.jar -checkVer
java -jar STLinkUpgrade.jar -update
```

这些命令已对本板实际执行成功。`-update` 为正常升级；不需要 `-force_prog` 或指定另一种探针功能类型。
`probe-rs info` 对本板不能自动识别型号，并且会忽略 `--chip`；确认 SWD 可用应使用显式目标的 read/run。
Cargo runner 已加入 `--speed 1000 --connect-under-reset --verify --no-catch-reset --rtt-channel-mode no-block-skip`；探针实际协商为 950 kHz。
`--no-catch-reset` 防止工具留下复位向量捕获，导致退出调试器后按 RESET 停在复位入口；它与避免 RTT 阻塞是两项独立设置。

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

固件启用 `defmt-rtt/disable-blocking-mode`，避免调试工具把 RTT 改成阻塞模式后，日志缓冲区满时等待读取者并长时间屏蔽 USB 中断。
runner 的 `--rtt-channel-mode no-block-skip` 仍保留；独立运行的音频不能依赖调试器持续读取日志。无读取者时日志可能被覆盖，RTT 日志不保证完整。
2026-11-05 此修改已烧录并完成独立 10 秒录音，以及强制 RTT mode=2、无日志读取者的 60 秒录音；两次均完成且启动 200 ms 后全零毫秒数为 0。
60 秒回归的 Windows 状态轮询未观察到设备变化；原先报告的间歇性设备消失仍待捕获。映像校验和、测试条件、结果及局限见 [非阻塞日志回归记录](../artifacts/usb-nonblocking-validation.json)。本次新映像的录音覆盖时长为 10 秒和 60 秒，阶段四的 600 秒记录对应历史映像。

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

## 默认真实麦克风录音

Windows 设置 → 隐私/隐私和安全性 → 麦克风：允许桌面应用访问。
在声音输入或经典 `mmsys.cpl` 的录制设备中选择本设备，尽可能选择 1 channel、16-bit、16000 Hz；关闭增强、降噪、AGC、通信衰减和监听。
本固件未暴露音量控制，只暴露主通道 Mute；不要以 Windows 是否出现音量滑块判断枚举成败。

```powershell
python tools/record_windows.py --list
python tools/record_windows.py --device STM32F4 --seconds 10 --output captures/microphone.wav
python tools/analyze_microphone.py captures/microphone.wav --json captures/microphone-analysis.json
```

录音脚本使用 Windows winmm 和 Python 标准库，无需 pip 包；仅选择名称匹配且唯一的输入设备，拒绝无匹配或歧义匹配。输出 WAV 和同名 `.capture.json` 元数据。保持固件运行，在另一个 PowerShell 窗口执行录音命令。

录音时说话或拍手，回放确认声音响应；分析结果检查时长、峰值、DC、AC RMS、削顶及启动后全零毫秒。室内背景声的幅度随环境变化，不使用阶段一正弦波的峰值/THD 作为真实麦克风门槛。必须保留原 WAV 才能复核，分析值不能单独证明无丢帧或标定声学性能。

默认数字增益 4×；USER 短按松开循环 4× → 16× → 1×，重启恢复。降噪版按住 1～3 秒松开切换旁路，至少 3 秒松开重新校准。可以分别录音比较幅度及削顶；Windows 主通道静音应使输出为零，恢复后应继续响应。实体按钮和静音操作仍待手工验收，算法测试通过不能代替这两项。

### 软件正弦诊断

需要隔离麦克风/DSP 问题时，先烧录无采集的最小固件：

```powershell
cargo run --locked --release --no-default-features
```

在另一个窗口执行：

```powershell
python tools/record_windows.py --device STM32F4 --seconds 10 --output captures/tone.wav
python tools/analyze_tone.py captures/tone.wav --json captures/tone-analysis.json
```

预期 1000 Hz 主峰，峰值约 8192（-12.04 dBFS），RMS 约 5793（-15.05 dBFS），DC 接近 0，无削顶和启动后整毫秒静音。分析脚本报告 THD 和周期差异；完整周期丢失可能无法由正弦波发现。

要同时检查 PDM/PCM 后台处理，烧录 `cargo run --locked --release --no-default-features --features pdm-capture`；USB 仍为正弦。完成诊断后用 `cargo run --locked --release` 恢复真实麦克风。

## 稳定性复验与剩余验收

1. 连续录音 10 分钟：`--seconds 600`。此项已有阶段四历史通过记录；新固件复验仍需对比录音时长、实际耗时、波形和 RTT 计数。
2. 录音启动/停止 20 次；CN1 保持供电时 CN5 插拔 20 次；每次 Windows 都应能重新打开录音。
3. 再测试 Windows 睡眠/恢复、USB 端口或 Hub 切换。逐项记录操作、时刻、设备状态和故障，不把一次成功称为长期稳定。
4. 保留开启和结束时的 RTT；恢复后 packets 必须继续增加，LED 不能停止。

600 秒命令示例（先保持默认固件运行，另一个窗口录音）：

```powershell
python tools/record_windows.py --device STM32F4 --seconds 600 --output captures/stability.wav
python tools/analyze_microphone.py captures/stability.wav --json captures/stability-analysis.json
```

保存本次完整 RTT 为新的日志文件后再汇总，不要把仓库中的历史日志当作本次结果：

```powershell
python tools/summarize_rtt.py artifacts/retest-rtt.txt --json artifacts/retest-rtt-summary.json --expect-clean
```

稳态检查 `usb_errors/ring_overruns/dma_errors/overflow_events/lost_samples/underruns/missing_samples` 不增长。`disabled/discarded_samples` 可因正常关流增加；预填补零、空闲丢弃和重置清空单独统计。完整含义见 [RTT 字段表](architecture.md#rtt-字段与边界)。计数器复位、回绕或 RTT 丢日志都可能影响判断。

三次顺序重开不等于 20 次启停；复位不等于断电启动或 CN5 拔插。2026-11-10 已在加入间断通知的自动降噪版上完成 20 次录音启停，见 [音频间断验收](audio-discontinuity.md)；20 次物理拔插、睡眠恢复、不同端口、Windows 11 和 8 小时录音仍待系统完成，应逐项记录。

## 出问题时的观察顺序与回传资料

### 间歇性设备消失

先在不连接调试器的情况下运行 Windows 枚举监测，再正常使用麦克风：

```powershell
powershell -NoProfile -File tools/watch_usb.ps1 -Seconds 1800
```

脚本不录音、不复位、不修改电源设置，在 `captures/usb-watch-*.jsonl` 保存带本地时区的初始状态及变化。
同时跟踪麦克风 USB 父设备、音频接口/端点、ST-LINK 和已发现的上游 USB Hub。
默认在每次查询后等待 1 秒；很短的断开重连可能被漏过。`complete` 只代表监测结束，不代表设备无故障；查询失败会记录 `error` 并退出，不伪报拔出。

如果只有音频端点消失而 USB 父设备仍在，继续查 Windows 音频接口；如果 USB 父设备也消失，继续查 USB 链路、Hub、电源及 MCU 状态。
这些状态本身不能确定根因。掉线时记录绿色心跳灯是否仍每秒翻转，并尽可能在插拔/复位前保存状态；后续附加调试器也可能扰动实时采集。
若 CN5 经过外置 Hub，可在 CN1 供电保持不变时，单独将 CN5 改接主机直连端口作对照；再单独换数据线，避免一次改多个变量。

| 现象 | 具体观察 |
| --- | --- |
| probe-rs 拒绝连接 | 保存 `probe-rs --version`、`probe-rs list`、`probe-rs info` 完整输出及 ST-LINK 升级工具版本截图 |
| 有 boot、没有 clocks ready | MCU 停在 HSE/PLL 初始化；核对 X2、PH0/PH1、板号及是否改过焊桥；不要盲改成其他板的时钟 |
| clocks ready 后 Windows 无设备 | 核对 CN5 数据线、设备管理器新增项/错误码、PA9 VBUS；测 CN5 VBUS 对 GND，确认 PC0 高；保存 RTT |
| Code 43/描述符失败 | 保存 USBView 输出、Windows 设备状态、reset/configured 日志；先解决 EP0/时钟问题 |
| 能枚举但录不到 | 检查隐私权限、所选设备、alt=1 是否到来、packets 是否增长、mute 是否开启 |
| 杂音/间断/重连后不恢复 | 保存 WAV、capture JSON、analysis JSON、RTT 前后计数、Windows 版本和发生故障的精确操作 |

如需示波器检查，仅在排错阶段由用户操作：8 MHz 晶振应使用适合的高阻探头以免扰动。启用 `pdm-capture` 时 PB10 名义时钟为 1.024 MHz；只有 `--no-default-features` 且未启用采集时不输出 PDM 时钟。物理频率精度尚未测量；正常使用无需外接采集模块。
