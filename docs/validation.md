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

- Flash 加载映像范围：40592 bytes（约 39.64 KiB，含初始化数据）。
- 静态 SRAM section 合计：4084 bytes（约 3.99 KiB）。
- `.data` 360 bytes；`.bss` 2700 bytes；`.uninit` 1024 bytes。
- 所有静态 RAM 从 `0x20000000` 开始，未分配 CCM。
- Cortex-M 栈的运行时高水位未测量，4084 bytes 不等于总运行时 RAM 峰值。

ELF 和 BIN 位于 `artifacts/stage1.elf` / `artifacts/stage1.bin`；精确链接布局见 `artifacts/firmware.map`。
Flash/RAM 数值只描述本次构建，不能据此推断后续 PDM 解码/USB 实时性能。

## 实体板检查与阻碍

用户确认：MB997B，CN5 OTG 已连接电脑。
Windows 检测到 `USB\VID_0483&PID_3748`，设备名 `STM32 STLink`，状态 OK。
probe-rs 0.32.0 能列出 STLink V2，但 `info` 被探针旧固件阻挡，提示最低支持版本 26。

原始输出：[artifacts/probe-info.txt](../artifacts/probe-info.txt)。
Windows 音频输入设备枚举保存于 [artifacts/windows-audio-devices.json](../artifacts/windows-audio-devices.json)，当时未出现本项目 STM32F4 USB 音频设备；该结果发生在尚未烧录新固件之前。

用户随后授权自动升级 ST-LINK，并已同意 STSW-LINK007 的 SLA0048 许可。
ST 官方页面当前提供 3.18.12，访客下载需姓名和邮箱收取下载链接；正在等待用户提供或完成下载表单。
尚未下载该升级包，尚未更改 ST-LINK 固件，也未执行目标 MCU 烧录。

| 实体检查 | 状态 |
| --- | --- |
| ST-LINK 在 Windows 可见 | 已观察 |
| ST-LINK 固件升级 | 待获取官方包 |
| 连接并确认 MCU 身份 | 未通过，受旧探针固件阻挡 |
| 应用固件烧录/校验 | 未执行 |
| 板上 8 MHz / USB 48 MHz 时钟 | 配置已审查，未实测 |
| USB UAC1 枚举 / 系统 usbaudio.sys 绑定 | 未验证 |
| 16 kHz / 16-bit / mono 线上描述符 | 源码配置已审查，未由主机读取验证 |
| 实际 1 kHz 录音、静音控制 | 未验证 |
| 连续录音、启动停止、插拔与睡眠恢复 | 未验证 |
| I²S2 + DMA、PDM 位流、PDM → PCM | 尚未实施 |

下一步先解决 ST-LINK 升级与目标连接，依照 [Windows 验证流程](windows-validation.md) 完成第一阶段硬件验收；未跳到第二阶段。
