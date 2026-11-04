# 第三阶段：PDM → 16 kHz PCM

2026-11-05，实体 MB997B。第二阶段通过并提交后，接入了静态状态、无堆分配的 Rust 解码器。

## 算法与可复现设计

1. 以每个 u16 的 bit15→bit0 处理 PDM，0→-1、1→+1。
2. 五级 CIC、16 倍抽取：1.024 MHz → 64 kHz。增益 16^5=1048576。
3. 191 抽头对称 Kaiser FIR（beta=7.8562，设计截止 7 kHz），再 4 倍抽取到 16 kHz。
4. 一阶 20 Hz 高通去直流，饱和转换为 i16。

每 4 个 PDM 半字固定输出 1 个 PCM 采样，总抽取比 64；128 半字 DMA 块固定产生 32 个 PCM 采样。
CIC 积分器与梳状差分显式使用 wrapping 算术，长期积分的模 2^32 回绕是算法的一部分，不能用普通溢出 panic 或饱和运算代替。
发生 DMA 覆盖后清空环形缓冲区并重置滤波状态，记录未知丢失事件，不跨缺口保持旧历史。

FIR 系数由 `tools/design_filter.py` 用 Python 标准库生成，先舍入到 f32，再输出可精确往返的最短十进制常量。
生成的实际 f32 系数响应：0～6 kHz FIR 本身误差约 -0.00089～+0.00043 dB，8～32 kHz 最大约 -80.68 dB。
这仅是 FIR 的响应；CIC 在 6 kHz 有约 0.6 dB 下垂，高通在低频有额外衰减，不把 FIR 单独指标说成整条链的指标。
线性级群延迟约 1.52 ms，高通另有频率相关延迟；未做声学延迟标定。

## 主机自动测试

构建、Rustfmt、Clippy `-D warnings`、9 项主机测试通过。测试包括原 3 项 USB 软件音源测试及：

- 100000 个连续输入半字严格保持 4:1 输出数量。
- 平衡位流、非零直流密度、积分器回绕后均无持续伪直流。
- 1 kHz、幅度 0.25 的合成 PDM：输出基频幅度约 8205.17 LSB、DC≈-0.004 LSB、SNR≈43.79 dB。
- 6 kHz 同幅输入输出约 7641.36 LSB；10 kHz 输入在 6 kHz 的混叠分量约 2.63 LSB，满足测试的 60 dB 抑制门槛。
- 实际 f32 FIR 系数阻带超过 70 dB；重置后的解码与新实例一致。

测试 PDM 由独立的一阶 sigma-delta 源生成，并使用固定种子的量化器抖动避免测试源自身的确定性闲置音调。
SNR 数字包含测试调制器噪声，不能当作实体 MP45DT02 的信噪比或产品性能。
记录见 `artifacts/stage3-host-tests.txt`、`artifacts/stage3-filter-design.json`、`artifacts/stage3-checks.txt`。

与 MCU 完全相同的库可在 Windows 上执行：

```powershell
cargo test --locked --lib --target x86_64-pc-windows-msvc -- --nocapture
cargo run --locked --example decode_pdm --target x86_64-pc-windows-msvc -- captures/stage2-pdm.u16le captures/decoded.wav
```

已将第二阶段 65536 位实体快照解码为 1024 个 PCM 采样，数量与 64 倍抽取一致。

## 实时性能失败与修复

初版循环 FIR 的环形索引逐抽头分支，加上 release 的 size 优化，在实体板上观测到每个 2 ms 输入块最长处理约 1941 us，持续出现 ring_overruns。
这一版本未作为通过结果，未进入第四阶段；原始失败日志保存于 `artifacts/stage3-overload-before.txt`。

修复包括：镜像环形历史使 FIR 窗口连续、移除热路径逐抽头回绕分支、release 改为 opt-level=3；每个有界 DSP 块后主动 yield，让 USB/控制任务运行。
主机测试重新通过后再次上板，观察到同一块最长约 870 us，采集速率恢复为每秒 1024000 位、PCM 每秒 16000 samples，DMA 硬件错误和环形覆盖均为 0。
这是观察到的最长值，不是所有负载下的 WCET 证明。

## 实体解码与声音响应

启动约 8 秒后采集一份相互对应的原始 PDM / MCU 解码 PCM 快照，各 64 ms。快照均已完整提取。
测试期间通过 Windows 默认音频输出播放 1 kHz、数字峰值约 -26.2 dBFS 的短测试声，未使用外接采集模块。
MCU PCM 快照 1024 samples，频谱主峰 1000 Hz（频率网格间距 15.625 Hz），基频幅度约 14.90 LSB；峰值 49、AC RMS≈20.42、DC≈-2.08 LSB、削顶 0。
这说明板载麦克风到解码器的最小声音响应存在；播放声压、距离、扬声器与麦克风增益未标定，不能从此快照推导频响或灵敏度。

此外，在解码持续运行时，3 秒 USB 软件正弦波录音再次通过；此阶段 USB 仍发送正弦波，尚未发送实体 PCM。
实测内存：静态 SRAM 19240 bytes、Flash 映像 62520 bytes；全部 RAM 在 SRAM1/2，栈高水位未测量。

证据：`artifacts/stage3-rtt.txt`、`artifacts/stage3-pdm-analysis.json`、`artifacts/stage3-mcu-pcm-analysis.json`、`artifacts/stage3-usb-tone-analysis.json`、`artifacts/stage3-memory.json`。

```powershell
python tools/extract_pcm.py artifacts/stage3-rtt.txt --wav captures/stage3-mcu.wav --json artifacts/stage3-mcu-pcm-analysis.json
```

快照仅验证有限窗口；长期稳定性、完整频响与物理位时钟精度仍未测量。
