# 资料核对与设计依据

查阅日期：2026-11-05（Asia/Tokyo）。用户确认板号为 MB997B、OTG CN5 已接电脑。

本文保留初始资料核对与设计决策，外部资料未因文档整理而重新查阅。固定 revision 的源码链接是依赖依据；“当日”“第一阶段”“后续”描述当时状态，当前实现以 [架构说明](architecture.md) 与 [阶段四报告](stage4-live.md) 为准。

## Embassy 依赖

所有直接使用的 Embassy crate 固定同一官方提交：
`9f9f1f1bdc6775347b9f3119921da65d56ca3651`（2026-11-04）。传递依赖由 `Cargo.lock` 固定。

当日 crates.io 的 `embassy-usb 0.6.0` 只有 UAC1 speaker，没有用户指定的 `class::uac1::source::AudioSource`。
上游当前源码已增加 source，并把模块移到 `class::uac::device::source`。
因此使用固定 Git 提交中的 `AudioSource`，没有重写 USB Audio Class。
当前包版本号仍显示 embassy-usb 0.6.0、embassy-stm32 0.6.0、embassy-executor 0.10.0；仅看版本号不足以区分 API，须同时看 revision。

已读取的官方资料与代码：

- [Embassy Book：USB 排错与配置](https://embassy.dev/book/#_the_usb_examples_are_not_working_on_my_board_is_there_anything_else_i_need_to_configure)
- [AudioSource / Config / Stream 源码](https://github.com/embassy-rs/embassy/blob/9f9f1f1bdc6775347b9f3119921da65d56ca3651/embassy-usb/src/class/uac/device/source.rs)
- [UAC1 共享描述符与控制处理](https://github.com/embassy-rs/embassy/blob/9f9f1f1bdc6775347b9f3119921da65d56ca3651/embassy-usb/src/class/uac/device/function.rs)
- [官方音源示例（只参考音频类用法，未照搬 F1 时钟和引脚）](https://github.com/embassy-rs/embassy/blob/9f9f1f1bdc6775347b9f3119921da65d56ca3651/examples/stm32f1/src/bin/uac_source.rs)
- [F4 OTG Driver 构造函数](https://github.com/embassy-rs/embassy/blob/9f9f1f1bdc6775347b9f3119921da65d56ca3651/embassy-stm32/src/usb/otg.rs)
- [F2/F4/F7 RCC 实现](https://github.com/embassy-rs/embassy/blob/9f9f1f1bdc6775347b9f3119921da65d56ca3651/embassy-stm32/src/rcc/f247.rs)

核对后的实际签名：

```rust,ignore
SourceConfig::new(sample_rates_hz, sample_width, channels)
AudioSource::new(&mut builder, state, source_config)
Driver::new_fs(peripheral, dp, dm, interrupts, ep_out_buffer, driver_config)
stream.wait_connection().await
stream.write_packet(packet).await
```

该音源描述符为 UAC1 / bcdADC=0x0100；拓扑为 Microphone Input Terminal → Mute Feature Unit → USB Streaming Output Terminal。
仅一条采样率 16000、一声道、16-bit；IN 等时异步端点间隔 1 帧。
上游按“每帧采样数加一个采样”的余量分配端点，因此 `wMaxPacketSize=34`，本阶段正常 payload 是 32 bytes，二者不矛盾。
源设备没有单独 feedback 端点。第一阶段软件波形随 USB 帧节拍发送；真实采集时钟与 USB SOF 独立，需要时钟漂移补偿。第四阶段最终采用分数重采样与固定 16-sample 包，15/16/17 变长包仅是未成功的早期尝试，不能当作当前策略。

## 板级原理图

- [ST MB997 B-02 官方原理图，共 6 页](https://www.st.com/resource/en/schematic_pack/mb997-f407vgt6-b02_schematic.pdf)
- [ST UM1472 官方用户手册](https://www.st.com/resource/en/user_manual/dm00039084-discovery-kit-with-stm32f407vg-mcu-stmicroelectronics.pdf)
- [STMPS2141 数据手册：STMPS2141STR 为低有效使能](https://www.st.com/resource/en/datasheet/stmps2141.pdf)
- [RM0090：SRAM/CCM 与总线架构](https://www.st.com.cn/resource/en/reference_manual/dm00031020-stm32f405415-stm32f407417-stm32f427437-and-stm32f429439-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)

原理图通过官方网页的 PDF 提取内容核对；本机直接从 ST 下载 PDF 时连接被重置，未生成可供本地视觉复核的 PDF 文件。
MB997B 的完整 B-01/B-02 后缀尚未由用户确认；本阶段涉及的晶振、USB、麦克风按 B-02 图设置，实测前仍应核对实板元件/焊桥是否改装。

| 原理图位置 | 板上信号 | 本阶段配置 |
| --- | --- | --- |
| sheet 3 | X2 8 MHz，PH0/PH1 | HSE Oscillator；不使用 Nucleo 式 HSE Bypass |
| sheet 5 / CN5 | PA11 OTG_FS_DM，PA12 OTG_FS_DP | 内部 FS PHY，设备模式 |
| sheet 5 | PA9 VBUS_FS | 配为无上拉输入，开启硬件 VBUS detection |
| sheet 5 | PC0 → U6 STMPS2141STR 的低有效 EN | PC0 高电平，关闭主机供电开关 |
| sheet 5 | PA10 USB ID | 设备模式不启用 OTG 主机切换 |
| sheet 4 / U9 MP45DT02 | PB10 CLK_IN、PC3 PDM_OUT | 当前由 I²S2 采集；第一阶段不驱动 |
| sheet 6 | PD12 绿色 LED | 每秒翻转，证明 executor 有进展 |

供电约定：CN1 供电，CN5 用于 USB 音频数据和 VBUS 检测。CN5 不是本固件的板级供电来源。
USB 描述符设 `self_powered=true`、CN5 `max_power=0`；板上 VBUS 检测/指示电路仍有微小负载，该配置不构成 USB 电气认证。

PLL：8 MHz / M=8 × N=336 = 336 MHz VCO；P=2 得 168 MHz SYSCLK；Q=7 得 48 MHz USB。
AHB=168 MHz、APB1=42 MHz、APB2=84 MHz；TIM2 由 HAL 作为 Embassy 时间驱动。
第一阶段没有启用 PLLI2S。当前采集模式共享 PLLM=8，以 PLLI2SN=256、PLLI2SR=5 得到 51.2 MHz 内核时钟；I²S 分频 50，名义 PDM 位速率 1.024 MHz，物理精度尚未独立测量。

RAM：SRAM1 112 KiB + SRAM2 16 KiB = 128 KiB，位于 `0x20000000..0x20020000`。
CCM 64 KiB 位于 `0x10000000`，DMA1/2 不可访问。`memory.x` 不声明 CCM；`inspect_elf.py` 检查全部已分配 section 只处于允许的 Flash/SRAM 区域。
第一阶段没有 DMA，这只能验证当前布局，不能替代第二阶段的 DMA 缓冲区地址与对齐检查。

## ST 官方音频 BSP：后续采集依据

固定 ST BSP 提交：`7576ab2dae717977839394a71006514cf28af831`。

- [stm32f4_discovery_audio.h](https://github.com/STMicroelectronics/stm32f4discovery-bsp/blob/7576ab2dae717977839394a71006514cf28af831/stm32f4_discovery_audio.h)
- [stm32f4_discovery_audio.c](https://github.com/STMicroelectronics/stm32f4discovery-bsp/blob/7576ab2dae717977839394a71006514cf28af831/stm32f4_discovery_audio.c)
- [ST AN3998：Discovery 音频播放与录音](https://www.st.com/resource/en/application_note/dm00040802-audio-playback-and-recording-using-the-stm32f4discovery-stmicroelectronics.pdf)

已核对 BSP：I²S2 使用 SPI2；PB10/PC3 为 AF5；RX 用 DMA1 Stream3 Channel0、半字宽度、循环模式。
`I2S2_Init` 为 Master RX、16B、LSB standard、CPOL HIGH、MCLK disabled，`AudioFreq=2*PCM AudioFreq`。
这些 I²S 配置承载的是连续 PDM 位流，不表示麦克风直接输出 16-bit PCM。
`PDMDecoder_Init` 使用 LSB 位序、小端、64 倍抽取、每毫秒 AudioFreq/1000 个 PCM 输出；ST 库使用 CRC 外设并带独立的库/许可要求。
本项目现已实现并测试 Rust CIC/FIR/DC blocker，见 [第三阶段](stage3-decoder.md)；未使用 ST 二进制滤波库。

BSP 的默认 32 MHz I²S 时钟与整数分频并非严格 1.024 MHz PDM 位时钟；不能把注释中的 16 kHz 当成已测得采样率。
第二阶段已核对 HAL 的 WS 要求、分频及位序，并完成 DMA 活动位流验证；第三阶段进一步验证声音响应。PB10 物理位时钟的仪器测量仍未完成，不能把配置值当作测量结果。
