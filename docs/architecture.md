# 当前固件架构

依据当前源码整理，2026-11-05。历史实测结果见 [第四阶段报告](stage4-live.md)，操作见 [Windows 指南](windows-validation.md)。

## 模式与模块

| 模块 | 职责 |
| --- | --- |
| `src/main.rs` | USB 描述符、音源任务、执行器、中断优先级和启动 |
| `src/board.rs` | HSE、主 PLL、PLLI2S 和总线时钟 |
| `src/pdm.rs` | I²S2/DMA、原始位流和 PCM 快照、采集诊断 |
| `src/decoder.rs` / `src/fir_coeffs.rs` | CIC5、191 tap FIR、去直流与 i16 输出 |
| `src/denoiser.rs` / `src/denoiser_tables.rs` | 无堆分配的 256 点 FFT、Wiener 降噪、现场噪声谱校准和固定延迟 |
| `src/noise_tracker.rs` | 可选 `adaptive-noise` 的初始谱参考、粗 / 慢频谱保护与有界背景谱更新；已完成短时人声实板复验 |
| `src/calibration.rs` | 分段暂存校准谱、谱结构 / 能量检查、丢弃不可信片段和安静等待 |
| `src/noise.rs` / `src/perf.rs` | 降噪按钮/LED、状态与 DWT 处理工作量统计 |
| `src/level_meter.rs` / `src/level_led.rs` | 20 ms RMS、明暗映射与蓝色 PD15 / TIM4_CH4 硬件 PWM |
| `src/pipe.rs` | 实时输入、USB 包准备、预填、静音、按钮和运行统计 |
| `src/audio_output.rs` | 队列间断序号、输出状态重置与重新预填；与主机恢复测试共用 |
| `src/audio_queue.rs` | 有界 PCM 队列、队列水位控制与饱和增益 |
| `src/resampler.rs` / `src/resampler_coeffs.rs` | 分数延迟重采样 |
| `src/stats.rs` | USB 状态回调与发送统计 |

默认启用 `live-mic` 和 `noise-suppression`，后者隐含 `live-mic`，再隐含 `pdm-capture`。仅启用 `live-mic` 是无降噪原声；仅启用 `pdm-capture` 时采集并解码，但 USB 发送软件正弦；禁用全部 feature 时仅发送正弦。模式选择命令见 [README](../README.md#构建与模式)。

## 时钟、引脚与数据路径

8 MHz HSE 晶振 → PLLM=8、N=336、P=2、Q=7 → SYSCLK 168 MHz、USB 48 MHz；APB1 42 MHz、APB2 84 MHz。启用采集时 PLLI2S N=256、R=5，得到 51.2 MHz I²S 内核时钟。

I²S2 Master RX 分频 50：PB10 输出名义 1.024 MHz 位时钟，PC3 接收 PDM，PB12 输出 HAL 要求的 WS（麦克风不用 WS）。DMA1 Stream3 Channel0 使用 SRAM1/2 内的 512 个 u16 循环缓冲区，按 64 个半字处理。

```text
MP45DT02 / PDM 1.024 MHz
  → I²S2 + DMA（512 半字，8 ms）
  → CIC5 /16（64 kHz）→ 191 tap FIR /4（16 kHz）→ 20 Hz DC blocker
  → Wiener 降噪（256 点窗 /128 点步进；默认 12 dB 上限；延迟 256 samples）
  → f32 PCM 队列（1024 samples；预填 256；目标 192）
  → 48 tap 分数重采样（32 相位间隔及插值）
  → Q8 饱和增益 → USB 主通道静音 → s16le USB IN
```

USB 每毫秒准备 16 samples /32 bytes，上游端点容量为 34 bytes。水位控制调整重采样输入步长，连接 MCU 采集时钟与主机 USB 节拍。当前实现不使用 15/16/17 samples 变长包；早期尝试和失败记录见阶段四。
降噪输出、PCM 队列和重采样保留 f32 的小数 LSB，仅在最终数字增益后量化到 i16；无降噪诊断构建保留原 i16 通路。
声音指示在采集侧取降噪 / 旁路后的信号，并计入当前 Q8 数字增益，使用独立蓝灯；未开启 USB 录音时也更新，不受 Windows 主通道静音影响。它不是标定声压级测量。

默认增益 4×，PA0 USER 短按切换 4× → 16× → 1×；降噪版新增长按旁路 / 重新校准，见 [降噪说明](live-denoise.md)。静音仍消费数据；关闭流、reset、断开或 suspend 清空队列，避免重新打开时播放旧音频。USB 音量控制未实现。

2026-11-10 增加统一音频间断通知：清空 PCM 队列时在同一短临界区递增 `generation`，包括空队列（重采样器仍可能保留历史）。DMA 覆盖在原有解码器 / 降噪历史重置前通知输出端；PCM 队列覆盖旧数据时也递增序号，并保留最新数据。输出端在包边界观察序号，重置水位控制、重采样相位和 FIR 历史，达到 256 samples 预填后恢复，等待期间发送 32-byte 零包。USB 流生命周期沿用同一通知路径；空闲采集不会每毫秒重复递增序号。

读取队列采样时校验所属序号；若包准备过程中序号变化，清零整个待发包并重置输出，且不消费新序号的数据。已经交给 USB 驱动的包不能追溯撤回；此机制不能补回已丢失的输入或保证切换绝无可闻瞬态。新增 8 个主机测试覆盖预填、旧历史清除、包内间断、空队列、溢出、序号回绕、新音频恢复和原有连续通路行为；2026-11-10 的三次真实 DMA 覆盖恢复及录音结果见 [音频间断验收](audio-discontinuity.md)，尚无主观无爆音验收。噪声谱保留策略、增益、采样率和包大小保持原设置，DMA 致命硬件故障的自动恢复另待实现。

## 调度与内存

音频发送使用 UART4 软件触发向量上的 InterruptExecutor（P6），可抢占主线程 DSP。OTG_FS=P2、DMA1_STREAM3=P3，数值越小优先级越高。UART4 外设未启用，其向量已被音频执行器占用。

采集、USB 总线控制、快照、按钮和诊断运行在主线程执行器；每个有界 DSP 块后主动让出执行权。FIR 计算保持中断开启，不在 await 时持有临界区。
降噪每 8 个 1 ms 解码块处理一次 FFT / IFFT，可被 USB 音频抢占。启动略过 504 ms，累计 500 个获准帧后完成校准；每段先暂存，发现活动会丢弃候选并延长等待，期间保持延迟一致的旁路。当前校准与慢频谱保护的证据边界见 [校准保护](calibration-guard.md)。DMA 覆盖时清空降噪时域历史，已完成的噪声谱保留。
解码器在 DMA 间断后保留慢变直流估计，清空捕获历史，预热 196 个 PDM 半字并对后续 64 个 PCM samples 淡入，避免直流状态清零产生恢复脉冲；实板及低音量人声测试见 [恢复瞬态优化](recovery-transient.md)。
RCC 初始化后开启 STM32F407 Flash 侧指令缓存、数据缓存和预取，保留 HAL 设置的等待周期；启动日志记录 `ACR before=5 after=705`。它不是 SRAM 的数据缓存，不需要对 DMA SRAM 缓冲区做缓存维护。

缓冲区静态分配，无动态堆；`memory.x` 仅声明 Flash 与 128 KiB SRAM1/2，排除 DMA 不可访问的 CCM。历史最终映像静态 SRAM 为 20688 bytes；这不包含已测得的栈峰值，栈高水位尚未测量。

## RTT 字段与边界

| 日志 / 字段 | 含义 |
| --- | --- |
| `stats packets` | USB API 成功接受的累计包数，不能证明主机收到每个包 |
| `usb_errors` | API 的 BufferOverflow 次数 |
| `disabled / discarded_samples` | 发送失败次数及对应丢弃采样；正常停止也可能各增加 1 /16 |
| `stats pending/high` | 待发送包采样数及最高值，不是 PCM 队列水位 |
| `pdm words` | 累计成功消费的半字数；稳态约每秒增加 64000 |
| `pdm ones/bits` | 每个诊断区间的 1 位数/位数；稳态 bits 约 1024000 |
| `pending_words/low/high` | 消费边界观察到的 DMA 半字积压与范围 |
| `ring_overruns / dma_errors` | 环形覆盖事件 / IRQ 锁存的硬件错误；覆盖的精确原始位数未知 |
| `pcm samples` | 每个诊断区间的解码输出数，稳态约 16000 |
| `pipe level/low/high` | PCM 队列水位，单位 samples |
| `overflow_events / lost_samples` | PCM 入队覆盖事件 / 被覆盖的旧采样数 |
| `underruns / missing_samples` | 输出欠载事件 / 缺失输出采样数；欠载补零后重新预填 |
| `priming_packets` | 启动/重新预填期间主动发送的零包 |
| `idle_discarded / reset_discarded` | 未录音时主动丢弃 / 关闭或重置时清空的 PCM 数 |
| `generation / interrupted_packets` | PCM 队列间断序号 / 因包准备过程中序号变化而整包清零的次数；正常 USB 启停也改变序号，不能直接作为故障数 |
| `rate_q16 / gain_q8 / clipped` | 重采样输入步长 / 增益 / 增益削顶数 |
| `decode_max_us / prepare_max_us` | 观察到的最长 DSP 块 / 包准备耗时，不能当作严格 WCET |
| `noise enabled/ready/calibration_frames` | 降噪使能 / 噪声谱是否就绪 / 已校准帧数，完成为 500 |
| `rejected_frames/hold_frames/calibration_blocked` | 被丢弃的候选校准帧 / 剩余安静等待帧 / 当前是否等待安静 |
| `noise hops/hop_max_us` | 128-sample hop 累计数 / 含抢占的最长观测耗时 |
| `level rms_lsb/brightness_permille/pwm_permille` | 20 ms 窗 RMS / 平滑后的相对亮度 / 蓝灯 PWM 占空比，后两项为 0～1000 |
| `level rms_high/pwm_high/updates` | 本诊断区间最高 RMS / 最高占空比 / 累计 20 ms 窗更新次数 |
| `work dsp_permille/audio_permille` | 对应诊断区间内解码及降噪 / 包准备周期占墙钟预算的千分比 |
| `work noise_subset_permille/accounted_permille` | 降噪子集开销 / DSP 与音频包准备合计；不是完整 CPU 占用率 |

`decode_max_us` 当前仅包围 PDM 解码，不再包含 PCM 发布和降噪；与旧版本字段相比时须注意计时范围。
DWT 为 32-bit，短跨度及每秒统计使用 wrapping 差值。DSP 计时扣除嵌套的音频包准备周期；降噪是 DSP 的子集，不可再次加到合计。
其他 IRQ 在被测工作期间的时间可能被包含，空闲区间的 IRQ、USB 控制及其他任务未完整覆盖，不能将 `accounted_permille` 称为精确整机 CPU 使用率。

`dma_active=false` 时无采集诊断，软件正弦模式也没有 `pipe` 水位。u32 累计计数会回绕、复位后清零；区间计数不能按累计值解释。DMA 致命硬件错误仍可能由 HAL panic，未实现所有故障的自动恢复。

`tools/summarize_rtt.py --expect-clean` 要求七个错误字段均存在且观察最大值为零。它适合检查完整的实时麦克风日志；缺字段、非零历史值、复位或 RTT 丢日志都需要结合原始记录解释。
