# STM32F4DISCOVERY USB 麦克风

Rust + Embassy，STM32F407VG / MB997B，板载 MP45DT02，无外接采集模块。
默认固件将真实麦克风送到 UAC1：16 kHz、16-bit PCM、单声道；Windows 原生 usbaudio 驱动。
默认启用 12 dB 上限的 Wiener 降噪。启动后先保持安静约 5 秒，红灯熄灭、橙灯亮后完成校准；使用与验证范围见 [板上降噪](docs/live-denoise.md)。
蓝色 PD15 LED 按当前声音强弱调节亮度，安静时较暗、说话时变亮；不要求 Windows 正在录音。四灯用途及规则见 [声音强度指示](docs/level-led.md)。

阶段一的 USB 正弦波、阶段二的 I²S2 DMA、阶段三的 PDM 解码均已在本机实体板验证并分别提交。
阶段四已完成 600 秒连续实体麦克风录音、停止后重开和复位后脱离调试器的 10 秒录音；可观察错误/丢样计数在连续测试中为 0。结果见 [实时麦克风验证](docs/stage4-live.md)。
本机为 Windows 10 Enterprise 19045；Windows 11 尚未实测。

## 从这里开始

1. 按下方说明连接 CN1、CN5，安装构建与调试工具。
2. 运行 `tools/check.ps1` 做离线检查，再用 `cargo run --locked --release` 烧录默认实时麦克风。
3. 在另一个 PowerShell 窗口录音，使用 `analyze_microphone.py` 分析真实麦克风数据。

完整操作见 [Windows 使用与排错](docs/windows-validation.md)；[文档索引](docs/README.md) 区分当前指南、实现说明和各阶段历史证据。

## 构建与模式

使用 Windows 原生 Rust MSVC 工具链、Visual Studio C++ Build Tools、Python 3、probe-rs，无需 WSL。
固定 Rust 1.99.1；Embassy 各直接依赖固定同一 Git 提交，保留 Cargo.lock。
上游当前实现的路径为 `embassy_usb::class::uac::device::source::AudioSource`；未重写 UAC。

```powershell
rustup target add thumbv7em-none-eabihf
cargo build --locked --release
.\tools\check.ps1
cargo run --locked --release
```

Cargo runner 使用 `STM32F407VGTx --speed 1000 --connect-under-reset --verify --no-catch-reset --rtt-channel-mode no-block-skip`。
需要将 probe-rs 加入 PATH；工具读取 RTT 不应使用 blocking 模式。若执行策略阻止脚本，可逐条运行其命令。
烧录会覆盖目标 MCU 应用。构建产物位于 `target/thumbv7em-none-eabihf/release/stm32f4-usb-mic`。

| 选择 | 命令 | USB 内容 |
| --- | --- | --- |
| 默认实时降噪麦克风 | `cargo build --locked --release` | Wiener 降噪，默认 4× 数字增益 |
| 原声实时麦克风 | `cargo build --locked --release --no-default-features --features live-mic` | 真实 PCM，无降噪 |
| PDM/PCM 独立诊断 | `cargo build --locked --release --no-default-features --features pdm-capture` | 1 kHz 软件正弦，后台采集/解码 |
| 第一阶段最小固件 | `cargo build --locked --release --no-default-features` | 1 kHz 软件正弦，不启动 DMA |

烧录对应模式时，将表中的 `cargo build` 换成 `cargo run`，保留全部 feature 参数。
`--features pdm-capture` 单独使用仍保留默认 `live-mic`；独立诊断必须同时指定 `--no-default-features`。

PA0 USER 短按后松开，增益切换 4× → 16× → 1× → 4×；按住 1～3 秒后松开，切换降噪 / 原声；按住至少 3 秒后松开，重新校准，随后安静约 5 秒。
重启恢复 4×、降噪开启并重新校准，不写 Flash。新长按控制已实现，实体按钮操作仍待验证。
支持 USB 主通道静音；不宣称支持尚未实现的 USB 音量控制。

## 接线与 Windows 录音

CN1 Mini-B 接 ST-LINK/供电，CN5 Micro-AB 用数据线接 Windows；保留 CN2 两个板内 SWD 跳帽。
本板 ST-LINK 已从 V2J14S0 升级至 2.49.0。CN5 VBUS 曾读低，重新插拔后恢复；PA9 电平持续写入日志。

```powershell
python tools/record_windows.py --list
python tools/record_windows.py --device STM32F4 --seconds 10 --output captures/microphone.wav
python tools/analyze_microphone.py captures/microphone.wav --json captures/microphone-analysis.json
```

脚本只录制名称匹配且唯一的 STM32F4 设备，使用 winmm，不需 pip 包。录音和元数据保存在本地 `captures/`，不纳入 Git。
`analyze_tone.py` 仅用于软件正弦模式；实体麦克风应使用 `analyze_microphone.py`。
Windows 音频引擎可能处理或重采样；WAV 文件格式不能单独证明线上描述符正确。

## 数据路径与调度

模块职责、时钟和日志字段见 [当前固件架构](docs/architecture.md)。

PB10 I²S2_CK → MP45DT02，PC3 I²S2_SD ← PDM，PB12 是 HAL 要求的未接麦克风 WS 输出。
HSE 使用本板 8 MHz 晶振，主 PLL 产生 168 MHz SYSCLK / 48 MHz USB；PLLI2S 产生 51.2 MHz 内核时钟，I²S 分频 50，PDM 名义 1.024 MHz。
DMA1 Stream3 Channel0，512 个 u16 循环缓冲区，读取每块 64 个半字（1 ms）。

PDM → CIC5 /16 → 191 tap FIR /4 → 20 Hz DC blocker → Wiener 降噪 → 1024-sample PCM 队列 → 分数重采样 → 饱和增益 → USB。
总抽取 64，标称输出 16 kHz。48 tap、32 相位间隔的分数延迟滤波器保持 USB 每包 16 samples / 32 bytes；通过队列水位补偿 MCU/主机时钟差。

采集/DSP、USB 总线控制、诊断、快照和按钮使用独立异步任务。实时音频发送运行在更高优先级的 Embassy InterruptExecutor（仅软件触发 UART4 向量），能抢占 DSP；不占用 UART4 外设。
中断优先级 OTG_FS=P2、DMA1_STREAM3=P3、音频执行器=P6，PDM 在主线程执行器。
不在 await 期间持有锁，FIR 计算保持中断开启。所有 MCU 缓冲区静态分配，无动态堆；链接 RAM 仅 SRAM1/2，排除 CCM。

RTT 记录 USB 状态/包数/端点错误、DMA 硬件错误/环形覆盖、PCM 水位/溢出/欠载/丢样、启动补零、重连清空、增益削顶与处理耗时。
离线 ELF 检查由 `tools/inspect_elf.py` 完成；栈高水位和严格 WCET 未测量。

## 证据与局限

上述硬件结果来自仓库保存的 2026-11-05 验证记录；后续只改文档不构成新一轮实体测试。
`artifacts/` 保存版本化文本/JSON 证据；`captures/` 和 ELF/BIN/map 未纳入 Git，克隆仓库不会自动获得原始录音或固件二进制。

- [官方资料、API 与板级依据](docs/research.md)
- [第一阶段硬件记录](docs/validation.md)
- [第二阶段 PDM 采集与诊断](docs/stage2-pdm.md)
- [第三阶段滤波、PC 测试及实体快照](docs/stage3-decoder.md)
- [第四阶段实时麦克风与稳定性](docs/stage4-live.md)
- [离线降噪对比](docs/offline-denoise.md) / [板上降噪与处理预算](docs/live-denoise.md)
- [Windows 枚举、录音、恢复操作](docs/windows-validation.md)

本工程使用实验 VID/PID `c0de:cafe`，未经过 USB 认证；Windows 11、标定声学频响/灵敏度、长期温漂、完整电气测量仍未验证。
软件计数和有限时长录音不能证明绝无每一种线上 USB 丢包；报告会区分观察结果与尚未验证项目。
