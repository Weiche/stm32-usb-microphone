# STM32F4DISCOVERY USB 麦克风

Rust + Embassy，STM32F407VG / MB997B，Windows 10/11 原生 UAC1 驱动。

**本次交付为第一阶段：USB 软件正弦波固件。尚未通过实体板的烧录、枚举或录音验证。**
已检测到板载 ST-LINK/V2，但 probe-rs 拒绝连接：其固件低于最低支持版本 26。
详细记录见 [验证报告](docs/validation.md)，恢复步骤见 [Windows 操作说明](docs/windows-validation.md)。

## 当前实现

- 上游 `AudioSource` 实现 UAC1 麦克风；16 kHz、16-bit、小端有符号 PCM、单声道。
- 1 kHz 正弦波，峰值 8192（约 -12.04 dBFS），每 USB 帧 16 samples / 32 bytes。
- 原生 USB Audio 控制请求、alt 0/1 和音频描述符由 Embassy 实现；支持主通道静音。
- USB 总线、音频发送、RTT 诊断分为三个 async 任务；发送由端点反压驱动，不额外设 1 ms 定时器。
- 使用静态存储、无 MCU 堆分配；链接区仅含 Flash 和 128 KiB SRAM1/2，排除 CCM。
- RTT 输出 USB 状态、累计发送包、可观察错误、丢弃采样和待发送水位。
- 本阶段没有启用 I²S/DMA，也没有实现 PDM 滤波；`dma_active=false`，DMA 错误统计在第二阶段接入真实事件后增加。

## Windows 原生构建

需要 Rust 的 MSVC 工具链及 Visual Studio C++ Build Tools（主机测试链接器）。工程固定 Rust 1.99.1。
首次构建会下载固定版本工具链、Embassy 源码与 STM32 元数据，可能较慢；不依赖 WSL。

```powershell
rustup target add thumbv7em-none-eabihf
cargo build --locked --release
.\tools\check.ps1
```

如果 PowerShell 执行策略不允许运行脚本，可逐条执行其中命令。
首次从不带锁文件的源码初始化才需 `cargo build --release`；交付已包含 `Cargo.lock`。

构建结果：`target/thumbv7em-none-eabihf/release/stm32f4-usb-mic`（ELF）。

```powershell
python tools/inspect_elf.py target/thumbv7em-none-eabihf/release/stm32f4-usb-mic --bin artifacts/stage1.bin --json artifacts/memory.json
```

`artifacts/stage1.elf`、`artifacts/stage1.bin` 为本次构建产物，不纳入 Git；以源码和锁文件为准。

## 烧录与录音

1. CN1 接 ST-LINK/电源，CN5 用 USB 数据线接 Windows；CN2 的两个板内 SWD 跳帽保留。
2. 先解决旧 ST-LINK 固件连接问题，执行 `probe-rs info` 确认芯片。
3. `cargo run --locked --release` 烧录并查看 RTT。此操作覆盖目标 MCU 的应用程序。
4. Windows 应出现 `STM32F4 USB Microphone`；关闭音频增强，使用 16 kHz 单声道格式。
5. `python tools/record_windows.py --list`，再录制并分析：

```powershell
python tools/record_windows.py --device STM32F4 --seconds 10 --output captures/stage1.wav
python tools/analyze_tone.py captures/stage1.wav --json captures/stage1-analysis.json
```

Python 录音使用 Windows winmm，不需要 pip 包。它只会选择名称匹配且唯一的设备，匹配失败即退出。
Windows 音频引擎可能重采样或处理，不能用录音文件格式单独证明线上端点格式；须同时检查 USB 描述符。

## 工程导航

| 文件 | 用途 |
| --- | --- |
| `src/main.rs` | USB 构建、三个任务、静态缓冲区 |
| `src/board.rs` | MB997B 专用晶振与 PLL 配置 |
| `src/lib.rs` | PC 可测的正弦波与 PCM 小端编码 |
| `src/stats.rs` | USB 回调、可观察计数 |
| `memory.x` | 排除 CCM 的内存布局 |
| `docs/research.md` | 官方资料、固定提交、API 差异与后续 PDM 依据 |
| `docs/windows-validation.md` | 烧录、枚举、录音、故障定位与日志清单 |
| `docs/validation.md` | 本次实际检查结果及硬件限制 |

阶段状态：第一阶段代码交付；硬件验收待完成。第二至第四阶段未实施。第一阶段硬件不通过时，先解决枚举/传输问题，再进入采集阶段。

USB 使用 Embassy 示例的 `c0de:cafe`，只用于私人实验，不代表已分配的 USB 标识；商业发行需合法 VID/PID。
