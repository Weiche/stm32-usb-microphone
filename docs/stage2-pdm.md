# 第二阶段：I²S2 + DMA 原始 PDM

2026-11-05，实体 MB997B / STM32F407VG。阶段一的 USB 软件正弦波保留，采集与 USB 使用独立任务。

> 历史记录：本报告描述第二阶段构建，当时为 1024 半字 DMA /128 半字处理块。当前源码已改为 512 /64，默认 USB 输出真实麦克风，见 [当前架构](architecture.md) 和 [阶段四报告](stage4-live.md)。

## 配置依据

按已核对的 MB997 B-02 原理图、ST 官方 BSP 和固定 Embassy 提交实现，未套用其他开发板配置。
PB10=I2S2_CK、PC3=I2S2_SD，AF5；SPI2 Master RX、16-bit channel/data、LSB-justified、CPOL high、MCLK disabled。
PB12 为该 HAL 构造函数要求的 WS 输出，接扩展排针，麦克风不连接/不使用 WS，不需要外接模块。
“LSB-justified”是帧对齐方式，不代表串行数据先发最低位；本项目按每个接收半字的 bit15→bit0 处理位流。
ST BSP 先 HTONS 换字节再传其专用库；本项目不直接照搬库的内存位序标志。

主 PLL 和 PLLI2S 共享 PLLM=8。HSE 8 MHz / 8 × PLLI2SN 256 / PLLI2SR 5 = 51.2 MHz。
HAL I²S 的 frequency 参数设 32000，表示 32-bit 双声道帧率；分频为 50，PB10 名义位时钟 1.024 MHz。
它不是 32 kHz PCM；1.024 MHz / 64 = 16 kHz，供第三阶段抽取。
实板寄存器日志：I2SPR=0x19（DIV=25、ODD=0、MCKOE=0）、I2SCFGR=0xb28（启动前）。
未用示波器/逻辑分析仪独立测量晶振精度；DMA 进度只能验证相对于 MCU 时间基准的速率。

参考：[ST AN3998](https://www.st.com/resource/en/application_note/an3998-pdm-audio-software-decoding-on-stm32-microcontrollers-stmicroelectronics.pdf)、[Embassy 固定版本 I²S 源码](https://github.com/embassy-rs/embassy/blob/9f9f1f1bdc6775347b9f3119921da65d56ca3651/embassy-stm32/src/i2s.rs)。

## 静态内存、调度与错误

DMA1 Stream3 Channel0，半字宽度，1024 个 u16 循环缓冲区（16 ms）。硬件半传输/传输完成中断约每 8 ms 唤醒消费任务。
每次异步读取 128 个半字（2048 位，名义 2 ms）；只读取已经完成的 DMA 数据，无忙等。
实板 DMA 缓冲区地址 `0x20002f48`，运行时断言整个范围在 `0x20000000..0x20020000` 且半字对齐。
链接器继续排除 CCM。静态 SRAM 15472 bytes，Flash 映像 47264 bytes，栈高水位未测量；详见 `artifacts/stage2-memory.json`。

每秒报告成功消费半字总量、1 位数/总位数、可读半字水位、ring_overruns、dma_errors、SPI SR。
水位是在消费边界采样的可读积压，不是 DMA 每个周期的连续峰值；high 包括本次已读块。
HAL 遇 DMA TEIF 会 panic，因此 IRQ 前置处理先锁存并计数 TEIF/DMEIF/FEIF，再交给原 HAL，不伪装成已恢复错误。
环形缓冲区覆盖由 read 返回 Overrun，单独计数并清空对齐；精确丢失位数未知，不虚构丢样总数。
64 ms 原始快照在启动约两秒后采集一次；独立日志任务每 100 ms 输出一块，避免在采集热路径里大量打印。
快照中途发生覆盖会取消该快照；PC 工具拒绝缺块/乱序/未结束的转储。

## 本次实际验证

- 编译、Rustfmt、Clippy `-D warnings`、现有 3 项主机测试通过；日志 `artifacts/stage2-checks.txt`。
- `--no-default-features` 的阶段一兼容构建通过。
- 已上板烧录并读回校验，持续采集超过一分钟；DMA 硬件错误 0，环形缓冲区覆盖 0。
- 稳态每个约一秒诊断区间消费 1024000 位；DMA 水位 low=20、high=536（容量 1024 半字）。
- 完整快照 4096 半字 / 65536 位：1 位密度 0.5081787109375，2077 种半字值，46877 次相邻位翻转，原始检查通过。
- 重新插拔 CN5 后 USB 枚举恢复；同时采集 PDM 时，3 秒 USB 软件正弦波录音依然通过，usb_errors=0。

原始日志 `artifacts/stage2-rtt.txt`，快照分析 `artifacts/stage2-pdm-analysis.json`，USB 回归分析 `artifacts/stage2-usb-tone-analysis.json`。
快照实际数据为 `captures/stage2-pdm.u16le`（未提交 Git，保存在交付工作区）。
这些结果证明 DMA 可持续获取活动位流，不能独立证明麦克风频响、位序或声学保真；第三阶段还须验证滤波和声音响应。

## 复现与故障观察

```powershell
cargo run --locked --release --no-default-features --features pdm-capture
python tools/extract_pdm.py artifacts/stage2-rtt.txt --output captures/stage2-pdm.u16le --json artifacts/stage2-pdm-analysis.json
```

首条日志应显示 PDM DMA 地址在 0x2000xxxx、I2SPR=19；随后 words 持续增加。
若 words 不增，检查 DMA IRQ 和 I²S 启动；若数据全 0/全 1，检查 PB10 时钟、PC3 连通和麦克风供电，回传配置与原始日志。
如有仪器，PB10 应为约 1.024 MHz；该电气观测本轮未执行。
若 ring_overruns 增加，回传水位、USB 状态和 RTT 模式；保持 no-block-skip，不能用阻塞打印代替实时诊断。
若 `vbus=false`，先确认 CN5 仍连接；它与 PDM 位流是否活动是两个独立观察。

第二阶段当时默认 feature 为 `pdm-capture`，USB 仍发送软件正弦波。上述命令显式选择当前源码的独立诊断模式，不会恢复第二阶段历史缓冲区配置；重现原始版本需使用对应阶段提交。
