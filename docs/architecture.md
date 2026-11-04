# 当前固件架构

依据当前源码整理，2026-11-05。历史实测结果见 [第四阶段报告](stage4-live.md)，操作见 [Windows 指南](windows-validation.md)。

## 模式与模块

| 模块 | 职责 |
| --- | --- |
| `src/main.rs` | USB 描述符、音源任务、执行器、中断优先级和启动 |
| `src/board.rs` | HSE、主 PLL、PLLI2S 和总线时钟 |
| `src/pdm.rs` | I²S2/DMA、原始位流和 PCM 快照、采集诊断 |
| `src/decoder.rs` / `src/fir_coeffs.rs` | CIC5、191 tap FIR、去直流与 i16 输出 |
| `src/pipe.rs` | 实时输入、USB 包准备、预填、静音、按钮和运行统计 |
| `src/audio_queue.rs` | 有界 PCM 队列、队列水位控制与饱和增益 |
| `src/resampler.rs` / `src/resampler_coeffs.rs` | 分数延迟重采样 |
| `src/stats.rs` | USB 状态回调与发送统计 |

默认 `live-mic` 隐含 `pdm-capture`。仅启用 `pdm-capture` 时采集并解码，但 USB 发送软件正弦；禁用全部 feature 时仅发送正弦。模式选择命令见 [README](../README.md#构建与模式)。

## 时钟、引脚与数据路径

8 MHz HSE 晶振 → PLLM=8、N=336、P=2、Q=7 → SYSCLK 168 MHz、USB 48 MHz；APB1 42 MHz、APB2 84 MHz。启用采集时 PLLI2S N=256、R=5，得到 51.2 MHz I²S 内核时钟。

I²S2 Master RX 分频 50：PB10 输出名义 1.024 MHz 位时钟，PC3 接收 PDM，PB12 输出 HAL 要求的 WS（麦克风不用 WS）。DMA1 Stream3 Channel0 使用 SRAM1/2 内的 512 个 u16 循环缓冲区，按 64 个半字处理。

```text
MP45DT02 / PDM 1.024 MHz
  → I²S2 + DMA（512 半字，8 ms）
  → CIC5 /16（64 kHz）→ 191 tap FIR /4（16 kHz）→ 20 Hz DC blocker
  → PCM 队列（1024 samples；预填 256；目标 192）
  → 48 tap 分数重采样（32 相位间隔及插值）
  → Q8 饱和增益 → USB 主通道静音 → s16le USB IN
```

USB 每毫秒准备 16 samples /32 bytes，上游端点容量为 34 bytes。水位控制调整重采样输入步长，连接 MCU 采集时钟与主机 USB 节拍。当前实现不使用 15/16/17 samples 变长包；早期尝试和失败记录见阶段四。

默认增益 4×，PA0 USER 按钮切换 4× → 16× → 1×。静音仍消费数据；关闭流、reset、断开或 suspend 清空队列，避免重新打开时播放旧音频。USB 音量控制未实现。

## 调度与内存

音频发送使用 UART4 软件触发向量上的 InterruptExecutor（P6），可抢占主线程 DSP。OTG_FS=P2、DMA1_STREAM3=P3，数值越小优先级越高。UART4 外设未启用，其向量已被音频执行器占用。

采集、USB 总线控制、快照、按钮和诊断运行在主线程执行器；每个有界 DSP 块后主动让出执行权。FIR 计算保持中断开启，不在 await 时持有临界区。

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
| `rate_q16 / gain_q8 / clipped` | 重采样输入步长 / 增益 / 增益削顶数 |
| `decode_max_us / prepare_max_us` | 观察到的最长 DSP 块 / 包准备耗时，不能当作严格 WCET |

`dma_active=false` 时无采集诊断，软件正弦模式也没有 `pipe` 水位。u32 累计计数会回绕、复位后清零；区间计数不能按累计值解释。DMA 致命硬件错误仍可能由 HAL panic，未实现所有故障的自动恢复。

`tools/summarize_rtt.py --expect-clean` 要求七个错误字段均存在且观察最大值为零。它适合检查完整的实时麦克风日志；缺字段、非零历史值、复位或 RTT 丢日志都需要结合原始记录解释。
