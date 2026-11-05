# 文档索引

当前默认固件是 `live-mic` + `noise-suppression`：板载 MP45DT02 → Wiener 降噪 → 16 kHz、16-bit、单声道 USB UAC1。文档整理日期：2026-11-05（Asia/Tokyo）。

## 当前指南与实现

| 文档 | 用途 |
| --- | --- |
| [项目 README](../README.md) | 接线、构建、模式选择与快速录音 |
| [Windows 使用与验收](windows-validation.md) | 烧录、枚举、真实麦克风/正弦诊断、稳定性复验与排错 |
| [固件架构](architecture.md) | 当前模块、时钟、数据路径、调度和计数含义 |
| [离线降噪](offline-denoise.md) / [板上降噪](live-denoise.md) | 用户试听选择、算法对照、现场校准、处理预算及验证边界 |
| [声音强度指示](level-led.md) | 四个 LED 的分工、蓝灯 PWM 与相对音量映射 |
| [资料核对](research.md) | 固定依赖与官方板级依据；包含早期决策的历史背景 |

## 阶段验证记录

各阶段报告保留当时的参数、结果和失败记录；其中的默认 feature、缓冲区大小与测试数量不能直接当作当前版本配置。

| 阶段 | 验证范围 | 报告 |
| --- | --- | --- |
| 一 | USB 软件正弦、ST-LINK 升级、Windows 10 驱动与短时录音 | [第一阶段](validation.md) |
| 二 | I²S2 DMA 原始 PDM、快照提取、USB 正弦并行运行 | [第二阶段](stage2-pdm.md) |
| 三 | CIC/FIR 解码、合成 PDM 测试、实体声音响应和性能修复 | [第三阶段](stage3-decoder.md) |
| 四 | 实时麦克风、时钟桥接、600 秒录音、重开与复位后独立录音 | [第四阶段](stage4-live.md) |

第四阶段仍未验证实体按钮/Windows 静音操作、20 次启停/物理插拔、睡眠恢复、Windows 11、8 小时录音、标定声学性能、栈高水位和严格 WCET。完整边界以报告为准。

## 证据与本地文件

- [artifacts](../artifacts/)：提交到仓库的检查日志、RTT、录音元数据和分析 JSON；`stage4-*` 对应第四阶段，失败日志保留原名。
- [最终固件来源记录](../artifacts/stage4-firmware-provenance.json) 与 [SHA256](../artifacts/SHA256SUMS-stage4.txt)：描述当时实体测试映像，不能代替对新构建的校验。
- `captures/`：本地 WAV 和原始快照，未入 Git；分析 JSON 不能还原原始音频。
- ELF/BIN/linker map、`target/`：本地构建产物，未入 Git。历史报告中的这些链接在新克隆中可能不存在。

新一轮验证应使用新的文件名，保留模式、提交、工具链、系统、接线、RTT 和录音元数据，不覆盖历史证据。
