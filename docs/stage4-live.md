# 第四阶段：实时 USB 麦克风

2026-11-05，实体 MB997B / STM32F407VG，Windows 10 Enterprise 19045，系统 usbaudio 驱动。
默认 `live-mic` 已连接板载 MP45DT02 → PDM 解码 → USB；不依赖外接采集模块。已完成连续录音、重新打开音频流以及复位后的独立运行检查。

本文是 2026-11-05 保存的验证记录；文档更新不代表重跑硬件测试。当前操作见 [Windows 指南](windows-validation.md)，模块和日志字段见 [架构说明](architecture.md)。
文中的 `stage4-*` 证据文件位于 [artifacts](../artifacts/)；原始 WAV、ELF/BIN/map 不随 Git 交付。最终映像对应关系见 [来源记录](../artifacts/stage4-firmware-provenance.json) 和 [校验和](../artifacts/SHA256SUMS-stage4.txt)。

## 实现与故障边界

USB 总线使用上游 `AudioSource`；未实现第二套音频类。配置 16 kHz、s16le、单声道，端点最大包长保留上游的 34 bytes，实际每包固定 32 bytes / 16 samples。
采集使用 DMA1 Stream3 Channel0，512 个 u16 SRAM 循环缓冲区（8 ms）；每次读取 64 words（1 ms）并执行 CIC5 /16、FIR /4 和 DC blocker。

PCM 静态队列 1024 samples（64 ms），启动预填 256 samples，目标水位 192 samples。
队列满时保留最新数据，逐个统计被覆盖的旧 PCM samples；欠载时当前包剩余部分补零，记录缺失输出样本数，并重新预填及重置重采样器。
DMA 环形覆盖的实际原始位数不可知，单独计事件并重置解码器，不伪造精确原始丢样数量。
DMA 硬件错误在 HAL 前读取 TEIF/DMEIF/FEIF 并计数；HAL 的致命硬件错误仍会 panic，不宣称自动恢复所有 DMA 故障。

独立 MCU/USB 时钟由连续分数重采样连接：Q16 步长 65208..65864（约 ±5005 ppm），水位 EMA 32 帧，比例控制 4 Q16 units/sample。
48 tap Kaiser 分数延迟滤波器、32 相位间隔和相邻相位线性插值；没有用突兀的样本复制/丢弃代替正常时钟跟踪。
滤波器新增延迟 23/16000 s；总端到端延迟未测量。
`stage4-resampler-design.json` 基于实际 f32 系数：65 个相位点、0..6 kHz /20 Hz 网格，ASRC 单级幅度约 -0.01497..+0.00845 dB。
这不是板载麦克风的声学频响，也不包含 CIC/FIR/DC blocker/Windows 的响应。

数字增益采用 Q8 饱和计算，默认 4×（约 +12 dB），PA0 USER 按钮循环 4× → 16× → 1× → 4×，重启恢复 4×；统计增益削顶。
USB MUTE 时仍消费 PCM，避免暂停后队列积压。没有宣称实现 USB VOLUME 控制。
USB alt=0、总线 reset、断开和 suspend 会清空待录队列；新音频流重新预填，避免回放先前录音的积压。

## 调度修复与保留的失败证据

初次用 15/16/17 samples 的变长包补偿时钟，首次 34-byte 包后观察到发送不再前进；采集仍运行，队列满并丢样。
关联现象不等于已定位根因；日志为 `stage4-variable-packet-stall.txt`。
改为固定 32-byte 包和分数重采样后完成 12 秒录音，但线程执行器版本仍出现溢出；这不是通过结果。
日志为 `stage4-thread-executor-overflow.txt`，后续 60 秒尝试的日志为 `stage4-post-standalone.txt`。
该 60 秒尝试末尾重新附加调试器，因此不能作为全程无调试器的严格对照实验。

最终把音频发送放到 Embassy InterruptExecutor，以仅软件触发的 UART4 向量运行，优先级 P6；OTG_FS=P2、DMA=P3。
USB 控制、DSP、诊断等继续使用主线程执行器；音频可抢占有界 DSP 块，不在 await 中持有锁，也不在计算 FIR 时关闭中断。
更高优先级版本的首个 30 秒录音通过，所有可观察的 DMA/USB 错误、PCM 覆盖与欠载计数均为 0。

## 自动检查与实板结果

`tools/check.ps1` 已通过：release 构建、fmt、固件 clippy、Windows 主机测试和主机 clippy。
15 个主机测试覆盖精确抽取、频谱/幅度/DC、环形队列、增益饱和、相位滤波器，以及 ±5000/±1000/0 ppm 各 120 秒、4 ms DMA 突发模型下的水位。
重采样测试检查预期输出频率和输入消耗数量，避免仅凭 RMS 判定正确。
两种诊断模式也通过 release clippy，见 `stage4-compatibility-checks.txt`。

| 项目 | 状态 / 证据 |
| --- | --- |
| 高优先级版本 30 秒录音 | 480000 samples，0 削顶、0 个启动 200 ms 后的全零毫秒；`stage4-priority-30s-analysis.json` |
| 600 秒连续录音 | 9600000 samples /19200000 bytes，耗时 600.037 s，0 削顶、0 个启动 200 ms 后全零毫秒；`stage4-stability-600s-analysis.json` / `.capture.json` |
| 关闭后重新打开 | 顺序完成 30 s → 600 s → 12 s 三次录音；RTT 记录三次 alt=1，关闭后再打开均恢复 |
| MCU 复位后重新枚举与录音 | 退出 probe-rs，复位并重新枚举，独立完成 10 s /160000 samples；0 削顶、0 个启动 200 ms 后全零毫秒；`stage4-final-standalone-10s-analysis.json` |
| 实体 USER 按钮 / Windows 静音操作 | 尚未实测；算法测试及固件控制路径已实现 |
| Windows 11 / CN5 多轮物理拔插 | 尚未系统验证 |

长测与其前后两次录音期间，`usb_errors/ring_overruns/dma_errors/overflow_events/lost_samples/underruns/missing_samples` 全部保持 0；最终成功提交 USB 包累计 642063。
PCM 水位观察范围 145..272 samples；DSP 块最长观测 574 us（含被高优先级任务抢占的时间），音频包准备最长 158 us。
三次关闭各取消一个待提交包，因此 `disabled=3`、`discarded_samples=48`；队列主动清空累计 739 samples，均与持续运行丢样分开计数。
600 秒房间音频的一秒窗口 DC 为 -12.58..10.94 LSB、AC RMS 41.58..84.16 LSB，最大绝对采样值 448。
这些数字反映本次未标定的室内环境，不是麦克风灵敏度或噪声底指标。

最终 12 秒测试录音同时播放 8 秒 1 kHz 测试声。4..5 s 窗口在 980..1020 Hz /0.25 Hz 网格的峰值为 999.0 Hz，幅度约 75.90 LSB；停止测试声后 1 kHz 分量下降。
这是未标定的声学响应检查，不证明精确物理时钟或全频段性能。见 `stage4-stimulus-spectrum.json` 与 `stage4-final-stimulus-analysis.json`。
复位后的 10 秒录音全程没有 probe-rs 附加，未为此时段读取实时内部计数；其通过范围是枚举、录音完成与 WAV 指标。

静态 SRAM 20688 bytes，Flash 映像 73040 bytes；DMA 地址 `0x200046c0`，无 CCM 分配，见 `stage4-memory.json`。
这是静态分配及观察到的运行结果；栈高水位、所有负载下 WCET、精确端到端延迟尚未测量。
原始本地音频保存在 `captures/`，未加入 Git。报告中的数字不会把失败版本与最终版本混为一谈。

## Windows 复验与诊断

```powershell
cargo run --locked --release
python tools/record_windows.py --seconds 600 --output captures/stability.wav
python tools/analyze_microphone.py captures/stability.wav --json captures/stability-analysis.json
```

保持固件运行，在另一个 PowerShell 窗口录音。另将本次 RTT 保存为 `artifacts/retest-rtt.txt` 后汇总：

```powershell
python tools/summarize_rtt.py artifacts/retest-rtt.txt --json artifacts/retest-rtt-summary.json --expect-clean
```

仓库的 `stage4-rtt.txt` 是历史长测记录，可用于复算历史汇总，不能当作本次录音日志。

录音时每秒约 1000 个成功提交的 USB 包、16000 PCM samples、1024000 PDM bits。应检查 `overflow_events/lost_samples/underruns/missing_samples/ring_overruns/dma_errors/usb_errors` 不增长。
`priming_packets` 是启动有意补零；`idle_discarded` 是未录音时主动丢弃的实时输入；`reset_discarded` 是关闭/重置清空队列。
alt=0 时取消一个已经准备的 16-sample USB 包可能使 `disabled` 和 `discarded_samples` 增长，这是单独统计的流关闭行为。
这些计数不能证明每种 USB 总线丢包都被检测；端点 API 没有返回错误不等于绝无线上漏帧。

若 `vbus=false`，检查 CN5 的供电/数据线/接口并保存日志；不要关闭 VBUS 检测掩盖问题。
若 PCM 计数稳定、USB `packets` 停止而 stream=true，保存最后 30 秒 RTT、Windows 设备状态和录音脚本超时信息。
若 DMA 覆盖增加，应同时提供 `pending_words/high/decode_max_us/prepare_max_us`，说明是否开启调试器、快照或其他高负载任务。

可手工验证：录音中说话/拍手确认响应；停止再开始 20 次；CN1 保持供电而 CN5 拔插 20 次；记录每次枚举耗时与错误。
按 USER 分别检查 1×/4×/16× 幅度及削顶，切换 Windows 静音后检查数据持续而 PCM 为零，再恢复。
最后可执行 8 小时录音及休眠/唤醒、不同 USB 端口和 Windows 11 回归。未完成这些测试前不宣称产品级稳定性。
