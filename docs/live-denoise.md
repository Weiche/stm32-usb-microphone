# 板上 Wiener 降噪

2026-11-05，STM32F407VG / MB997B。用户选定离线较强档后，将同一算法移植为无堆分配的 Rust f32 实现。
离线听感与背景衰减记录见 [离线实验](offline-denoise.md)，逐样本对照见 [主机一致性记录](../artifacts/denoiser-host-parity.json)。

## 使用

默认 `cargo run --locked --release` 启用降噪。复位后，风扇 / 空调保持平常状态，安静约 5 秒：

- 红色 PD14 亮表示噪声校准中；略过启动 504 ms 后累积 500 个 8 ms 帧，共 4 秒。
- 校准完成且降噪开启时，橙色 PD13 亮；绿色 PD12 仍是每秒翻转的心跳。
- PA0 USER 短按松开继续切换增益 4× → 16× → 1×。
- 按住 1～3 秒后松开，在降噪 / 原声间切换；两者保持相同流水线延迟。
- 按住至少 3 秒后松开，重新校准；随后安静约 5 秒。

噪声谱只保存在 RAM，重启后重新学习。风扇转速或麦克风位置明显改变时可重新校准。
校准期间保持原声输出；校准不含自动语音检测，期间说话可能把人声学进噪声谱。
新增长按及指示灯路径已实现，实体 USER 操作尚待人工验证。

无降噪原声构建：

```powershell
cargo run --locked --release --no-default-features --features live-mic
```

## 算法与时序

PDM 解码后、增益和分数重采样前处理。256 点 FFT、128 点步进、周期 sqrt-Hann 分析 / 合成窗，参数与离线较强档相同：
Wiener 幅度增益最低约 0.25119（12 dB 上限），决策导向系数 0.96、三频点及时间平滑。
FFT 使用固定阶段和预计算 f32 系数；可用 `python tools/design_denoiser.py` 后执行 `cargo fmt --all` 重新生成表。

每次输入 / 输出均为 16 samples，内部每 8 次调用处理一帧。降噪 / 旁路均延迟 256 samples（16 ms）。
这只是本模块的流水线延迟，不是麦克风到 Windows 的完整端到端延迟。
FFT 工作保持中断开启，在主线程运行，UART4 软件执行器上的 USB 音频任务可抢占。
DMA 覆盖时重置时域历史；已完成的噪声校准保留，未完成的重新开始。

主机上将同一段 480000 samples 和同一噪声谱分别送入 Rust 与 Python：只有 6 个输出相差 1 LSB，其他完全一致。
旁路测试验证固定延迟和逐样本重建。现场校准直接使用增益前 PCM，因此不会硬编码离线 USB 录音的 4× 增益噪声谱。
降噪输出、队列与重采样之间保留浮点精度，最终数字增益后才量化到 USB i16，避免弱信号在增益前被截成零。

## Flash 加速与处理统计

本次发现固定版本 HAL 的 RCC 初始化仅写 Flash 等待周期，清除了缓存 / 预取使能。
应用在 RCC 初始化后开启 STM32F407 Flash 指令缓存、数据缓存和预取，保持原等待周期；启动实读 `FLASH_ACR: 0x005 → 0x705`。
Flash 缓存 / 预取的启用方式可参照 [ST 官方 F4 HAL 初始化](https://github.com/STMicroelectronics/stm32f4xx-hal-driver/blob/master/Src/stm32f4xx_hal.c)。

DWT 统计记录的是解码 / 降噪及包准备工作占 168 MHz 墙钟预算的比例：
DSP 扣除其被音频任务抢占的周期，降噪为 DSP 子集。其他任务和所有 IRQ 未全面计入，不能宣称整机平均占用率。
`hop_max_us` 包括抢占时间，是观察峰值，不是 WCET。

初版未启用 Flash 加速时，录音工作量约 88%；虽然固件错误计数为零，30 秒 WAV 中仍发现 251 ms 和 27369 ms 位置各一段全零毫秒。
这份初版诊断证据保存在本地 `captures/denoiser-live-30s*` 和 `denoiser-initial-rtt.txt`，不与后续版本混用。
仅修改 FFT 循环没有改善未缓存的实板开销；最终版本同时保留固定 FFT 阶段并开启 Flash 加速。
开启 Flash 加速但仍提前量化的版本，120 秒录音有 7 个全零毫秒、退出调试器后 60 秒有 3 个。
逐样本检查发现长零区间常不对齐 USB 包边界且邻接弱信号；不能仅凭这个指标认定 USB 丢包。
最终将降噪到增益之间的通路改为 f32；这些中间版本的证据保存在 `denoiser-final-120s*`、`denoiser-standalone-60s*` 和 `denoiser-final-rtt.txt`。

新映像的录音、资源、统计范围和局限见 [板上验证记录](../artifacts/denoiser-hardware-validation.json)。
最终 f32 通路版本连续录音 120 秒（1920000 samples /120.034 s），退出调试器后再次录音 60 秒（960000 samples /60.034 s）。
两次均无削顶、启动后 200 ms 以外的全零毫秒数均为 0。120 秒 RTT 中七项 DMA / USB / PCM 错误计数全部为 0；独立 60 秒期间未读取内部计数。
119 个稳态统计窗口中，DSP 约 42.5%、包准备约 8.5%，合计中位数 51.0%（范围 51.0～51.3%）；降噪为其中约 14.5% 的子集。
最长观测 FFT hop 1319 us，解码块 363 us，包准备 89 us。静态 SRAM 29048 bytes，Flash 映像 85712 bytes；栈峰值仍未测量。
原始 WAV、完整 RTT、ELF / BIN 在本地 `captures/`，未纳入 Git。历史阶段四的 600 秒测试对应无降噪旧映像。

## 实体 RESET 无响应的恢复

后续用户报告按 RESET 后无反应。现场仅有 ST-LINK，MCU 读到调试暂停状态，`DEMCR=0x01000401` 的复位捕获位开启；CFSR/HFSR 均为零。
清除 `DEMCR.VC_CORERESET` 并恢复执行后，未重刷固件即恢复 USB；工具退出后实读 `DEMCR=0x01000400`。
Cargo runner 新增 `--no-catch-reset`，防止后续调试运行保留复位捕获。
用户随后实际按 RESET，USB 于 23:58:15（Asia/Tokyo）重新枚举，15 秒无调试器录音完成，0 削顶、0 个启动 200 ms 后的全零毫秒。
这补充了先前仅验证“退出调试器后继续运行”的范围；本次只实测一次实体 RESET，尚未测试断电启动或多轮复位。
寄存器值、操作和录音证据见 [复位恢复记录](../artifacts/reset-recovery-validation.json)。

## 复验

```powershell
.\tools\check.ps1
python -m unittest discover -s tools -p test_denoise_offline.py -v
python tools/verify_denoiser_port.py
python tools/record_windows.py --seconds 120 --output captures/denoiser-retest.wav
python tools/analyze_microphone.py captures/denoiser-retest.wav --json captures/denoiser-retest-analysis.json
```

`verify_denoiser_port.py` 需要本地离线实验的原声、12 dB 参考输出和噪声谱；新克隆须先按离线文档重新录制并生成。
新增 5 项 Rust 检查涵盖 FFT 复数往返、旁路时延 / 精确重建、启动校准及复位语义、小数 PCM 队列 / 增益、重采样精度；与原 15 项一起运行。
