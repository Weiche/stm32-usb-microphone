# 第一阶段验证记录

日期：2026-11-05（Asia/Tokyo）。本报告明确区分离线检查与实体板验收。

## 已完成的离线检查

| 检查 | 实际结果 |
| --- | --- |
| Rust | 1.99.1，Windows MSVC 原生工具链 |
| MCU 目标 | thumbv7em-none-eabihf / STM32F407VG |
| `cargo fmt --all -- --check` | 通过 |
| `cargo build --locked --release` | 通过，生成 ELF 与 linker map |
| `cargo clippy --locked --release -- -D warnings` | 通过 |
| `cargo test --locked --lib --target x86_64-pc-windows-msvc` | 3 passed，0 failed |
| 主机测试 Clippy，`-D warnings` | 通过 |
| Python 三个工具的语法检查 | 通过 |
| WAV 分析器的合成正弦波输入 | 通过；peak=8192，RMS≈5792.584 |
| WAV 分析器的全静音负例 | 正确判为失败 |
| ELF section 地址检查 | 通过；全部已分配 section 只在 Flash 或 SRAM1/2，无 CCM |
| 链接后堆分配符号检查 | 未发现 `__rust_alloc`、`__rust_dealloc`、`__rdl_alloc`、`__rg_alloc` |

完整 Rust 检查输出：[artifacts/checks.txt](../artifacts/checks.txt)。第一次主机 Clippy 发现常量分块 API 建议，改用 `as_chunks::<2>()` 后重新执行全部检查并通过；未禁用该检查。

3 项 Rust 测试覆盖：USB 每帧采样数量/32-byte 小端封包；正负峰值、零直流和周期连接；1 kHz 对应 DFT 基频能量。
这些测试针对软件音源，不是第三阶段 PDM 滤波器测试；目前尚无 PDM 解码实现。

合成 WAV 仅是分析器的测试输入，保存在工作区 `work/algorithm-smoke`；不是开发板录音，也不作为硬件验收证据。

## 本次固件资源

来自 [artifacts/memory.json](../artifacts/memory.json)：

- Flash 加载映像范围：40688 bytes（约 39.73 KiB，含初始化数据）。
- 静态 SRAM section 合计：4084 bytes（约 3.99 KiB）。
- `.data` 360 bytes；`.bss` 2700 bytes；`.uninit` 1024 bytes。
- 所有静态 RAM 从 `0x20000000` 开始，未分配 CCM。
- Cortex-M 栈的运行时高水位未测量，4084 bytes 不等于总运行时 RAM 峰值。

ELF 和 BIN 位于 `artifacts/stage1.elf` / `artifacts/stage1.bin`；精确链接布局见 `artifacts/firmware.map`。
Flash/RAM 数值只描述本次构建，不能据此推断后续 PDM 解码/USB 实时性能。

## 实体板检查与实际录音

用户确认：MB997B，CN5 OTG 已连接电脑。
Windows 检测到 `USB\VID_0483&PID_3748`，设备名 `STM32 STLink`，状态 OK。
最初 probe-rs 0.32.0 能列出 STLink V2，但连接被探针旧固件阻挡，提示最低支持版本 26。

原始输出：[artifacts/probe-info.txt](../artifacts/probe-info.txt)。
Windows 音频输入设备枚举保存于 [artifacts/windows-audio-devices.json](../artifacts/windows-audio-devices.json)，当时未出现本项目 STM32F4 USB 音频设备；该结果发生在尚未烧录新固件之前。

用户授权自动升级 ST-LINK、同意 SLA0048 许可，并自行下载 STSW-LINK007 3.18.12 到 Downloads。
已检查 Windows EXE/DLL 的 Authenticode 签名，签名状态 Valid，签署者为 STMicroelectronics International N.V.。
原生 GUI 自动化因窗口捕获超时未能操作，未通过 GUI 执行升级；随后使用包内原版 Java 工具的命令行接口。
Java 运行时为工作区中的 Temurin 8u504，下载 SHA256 与 Adoptium 官方 API 提供值一致；未修改系统 PATH。

官方工具 `-list` 确认只有一只 ST-LINK/V2；`-checkVer` 读到 V2J14S0；`-update` 返回 `Upgrade is successful.`、`Version read: 2.49.0`。
原始记录：[artifacts/stlink-upgrade.txt](../artifacts/stlink-upgrade.txt)。没有使用强制重刷、改变探针类型或修改目标 option bytes。

升级后 `probe-rs info` 的自动识别仍失败；该子命令会忽略 `--chip`，不能将其结果等同于 SWD 连接失败。
使用明确芯片型号的 `probe-rs read` 成功读到 `DBGMCU_IDCODE=0x10016413`（DEV_ID=0x413，F405/407 器件组）、Flash 容量寄存器 `0x0400`（1024 KiB），与用户指定的 F407VG 一致。
此 ID 不能独立区分封装，具体型号仍结合用户板号和板载 MCU 标记判断。

已用 `probe-rs run --chip STM32F407VGTx --verify` 烧录并完成读回校验。RTT 显示：

```text
stage1 boot: MB997B STM32F407VG UAC1 16000Hz s16le mono sine=1000Hz
clocks ready: SYSCLK=168MHz USB=48MHz HSE=8MHz crystal
descriptors ready: config_bytes=119 packet=32 endpoint_max=34
stats vbus=false cfg=false suspend=false stream=false packets=0 usb_errors=0 ...
```

`clocks ready` 表示 HAL 已完成该时钟配置并返回，不代表使用仪器独立测量了晶振或 USB 时钟精度。
持续每秒日志证明调度器和诊断任务有进展；尚未收到 USB reset/configured 事件。
为定位原因，诊断任务增加 PA9 VBUS 输入日志；当前为低电平，Windows 仍未出现 VID_C0DE/PID_CAFE 音频设备。
用户重新插拔 CN5 后，日志变为 `vbus=true`，并出现 USB reset、configured=true 和 AudioStreaming alt 0。
Windows 设备管理信息显示 `STM32F4 USB Microphone` 与麦克风 AudioEndpoint 状态均为 OK。
初次 VBUS 低电平的具体原因未确定；不据此认定某根线缆或板上元件损坏。

退出调试后，普通重新连接出现 SwdDpWait；使用 `--connect-under-reset --speed 1000` 后成功重新烧录运行。
已将这一实测可用连接方式写入 Cargo runner，并使用 `no-block-skip` 避免 RTT 缓冲区满时阻塞固件。
这没有证明普通重连失败的根因，亦不把调试连接恢复当作 USB 恢复测试。

### Windows 实测

系统：Microsoft Windows 10 企业版，10.0.19045 / Build 19045。
音频驱动服务：`usbaudio`，INF：`wdma_usb.inf`，驱动版本：10.0.19041.5794。
记录见 `artifacts/windows-version.json`、`artifacts/windows-usb-devices.json`、`artifacts/windows-audio-driver.json`。
Windows 11 未实测，不将本机结果外推为两个系统均通过。

使用仓库内 Windows winmm 录音工具，明确选择名称含 STM32F4 的唯一设备，以 16000 Hz / 16-bit / mono 格式录制 10 秒，取得 160000 samples / 320000 PCM bytes。
原始文件：[captures/stage1-hardware.wav](../captures/stage1-hardware.wav)；录音 metadata 见 `artifacts/stage1-hardware.capture.json`。
这是实体开发板的 USB 录音，不是合成测试 WAV。

| 10 秒硬件录音指标 | 结果 |
| --- | --- |
| 主频（1 Hz 分辨率） | 1000 Hz |
| 峰值 | 8193 LSB，-12.0401 dBFS |
| RMS | 5792.593 LSB，-15.0515 dBFS |
| DC | -0.0005625 LSB |
| THD（2～7 次谐波） | 0.00486% |
| 削顶采样 / 启动后整毫秒静音 | 0 / 0 |
| 分析脚本判定 | 通过 |

详细结果：[artifacts/stage1-hardware-analysis.json](../artifacts/stage1-hardware-analysis.json)。
Windows 路径存在小幅采样变化：16-sample 周期比较中有 54344 个不完全相同的采样对，但绝对差最大仅 2 LSB，与音频转换/抖动相容；本次不宣称 bit-exact。
这些指标不是端到端丢包证明；重复正弦周期无法暴露所有整周期丢失。

录音过程中进入 alt=1，随后停止时回到 alt=0。结束时累计 packets=10021，usb_errors=0，disabled=1，discarded_samples=16，high=16。
其中 1 次 disabled 和 16 个未发送采样来自停止录音后待发送包失效，属于正常停止边界，不能报告为运行中 USB 错误或宣称整个端到端无丢样。

停止调试器、用 probe-rs 复位后，未保持 RTT 调试会话，再次独立录制 3 秒、48000 samples，分析通过；结果见 [artifacts/stage1-standalone-analysis.json](../artifacts/stage1-standalone-analysis.json)。
这验证了本次独立启动/重新打开录音的最小路径，不代替断电启动、20 次热插拔、10 分钟稳定性或睡眠恢复测试。

| 实体检查 | 状态 |
| --- | --- |
| ST-LINK 在 Windows 可见 | 已观察 |
| ST-LINK 固件升级 | 已成功：V2J14S0 → 2.49.0 |
| SWD 连接 / 器件组和 Flash 容量 | 已读取，符合 F407VG 配置 |
| 应用固件烧录/读回校验 | 已通过，两次构建均已运行 |
| HSE/PLL 初始化与 executor 诊断任务 | 已在实体板运行通过；频率精度未独立测量 |
| PA9 VBUS | 初次为低，重新插拔 CN5 后读到高电平 |
| USB UAC1 枚举 / 系统 usbaudio 驱动绑定 | 本机 Windows 10 已通过 |
| 16 kHz / 16-bit / mono 线上描述符 | 源码配置已审查，未由主机读取验证 |
| 实际 1 kHz 录音 | 10 秒及独立启动后 3 秒录音均通过 |
| 主通道静音控制 | 代码已实现，尚未单独执行硬件控制测试 |
| 连续录音、启动停止、插拔与睡眠恢复 | 仅上述短时路径，系统化稳定性测试未执行 |
| I²S2 + DMA、PDM 位流、PDM → PCM | 尚未实施 |

第一阶段最小 USB 音频路径已在本机通过。后续可依据 [Windows 验证流程](windows-validation.md) 补齐长期与跨系统验收，再进入第二阶段；本轮未实施第二阶段。
