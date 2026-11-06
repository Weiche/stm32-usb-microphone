# 自动跟踪背景噪声（可选实验）

启动校准后，只在一段时间未触发语音保护时缓慢更新噪声谱，尝试跟随适度的背景音量变化，减少重新校准次数。这是能量启发式，不是训练好的语音检测模型。固定噪声谱仍是默认配置。

## 构建和操作

```powershell
# 构建实验版
cargo build --locked --release --features adaptive-noise
# 连接开发板、准备好安静校准后烧录；会覆盖板上应用
cargo run --locked --release --features adaptive-noise
```

启动仍需安静约 5 秒，红灯熄灭、橙灯亮后完成初始校准。开机说话可能污染初始谱，本实验尚未解决这一问题。

USER 短按切换增益；按住 1～3 秒松开切换原声 / 降噪；至少 3 秒松开手动重校准。旁路暂停自动学习；恢复降噪、输入间断后都重新等待安静间隔。未打开 USB 录音时，采集和跟踪仍继续。

不带 `--features adaptive-noise` 构建、烧录即可恢复固定谱版。音频格式、USB 包大小、12 dB 降噪上限和 256 samples /16 ms 的降噪模块延迟不变。

## 更新规则

使用原始增益前的 FFT 功率，而非已经降噪的输出，避免衰减反馈。每个 128 samples /8 ms 的 hop 检查 250～4000 Hz（不含 4000 Hz）的 15 个 250 Hz 子带：

- 子带功率以 `0.9 * old + 0.1 * current` 平滑。
- 平滑子带超过初始校准的 5 倍、瞬时子带超过 20 倍、平滑总带功率超过 3 倍或瞬时总带功率超过 8 倍，均触发保护。
- 始终参照初始谱判断，避免估计值提高后逐渐放行持续人声。
- 保护解除后再等待 63 hops /504 ms，随后以 `N += 0.0016 * (power - N)` 更新；约 5 秒的时间常数只计算获准更新的帧。
- 每个频点限制在初始功率的 0.25～4 倍，并保留数值下限。约 ±6 dB 的范围只是估计边界，不代表所有范围内的背景变化都能通过保护。

复用原 FFT 和增益暂存数组，不增加 FFT 或动态堆。追加固定参考谱和少量状态；静态内存及已观测工作量见下方实板记录。

RTT 新增 `noise tracking=true speech_guard=... updating=... updates=... power_permille=...`。`speech_guard` 是能量异常标记，不是人声识别结果；`updating` 只描述最近一帧。`updates` 是最近校准以来的更新次数，u32 回绕。`power_permille` 是当前噪声谱功率和相对初始谱的千分比，1000 表示相同。每秒日志可能漏掉短暂事件，离线 CSV 保存每个 hop。

## 离线验证

```powershell
.\tools\check.ps1
python -m unittest discover -s tools -p test_denoise_offline.py -v
# 输出目录必须不存在，新克隆可只运行合成测试
python tools/validate_adaptive_noise.py --output-dir captures/adaptive-test
# 本机旧原始录音存在时，可追加真实房间音频
python tools/validate_adaptive_noise.py --output-dir captures/adaptive-room-test --room-wav captures/denoise-room-30s.wav
```

测试在主机运行与固件相同的 Rust DSP，覆盖独立噪声校准、噪声升降、持续调制谐波、低信噪比谐波和静音校准；输出固定谱 / 自动谱 WAV、逐帧 CSV。预计算谱的主机实验不模拟启动，启动 / 重校准 / 输入间断 / 旁路由 Rust 测试覆盖。

2026-11-06 首次实验中，合成背景幅度升至 1.5 倍（功率 2.25 倍）后，自动估计跟随至约 2.26 倍，最后 5 秒相对固定谱额外降低残留背景约 2.19 dB；幅度降至 0.65 倍时，估计跟随至约 0.42 倍。较强持续谐波期间没有更新噪声谱；这不是真人语音识别率。

旧房间录音 5～15 秒“说话请求区间”的整体电平相对固定谱变化约 -0.02 dB，25～30 秒“安静请求区间”约 -1.06 dB。区间依据旧录音约定，说话区间含停顿，没有干净语音参考，整体电平变化不能当成人声损失或音质证明。

完整数据、源码哈希及内存见 [实验记录](../artifacts/adaptive-noise-validation.json)。原始音频、WAV / CSV 和主机输出留在本地 `captures/`，不入 Git。

## 首次实板试运行

2026-11-06，按用户要求烧录 `adaptive-noise` 实验版并启用写入校验；启动日志明确给出实验跟踪已启用，完成 500 帧初始校准，Windows USB 枚举正常。

- 调试器连接时完成 30 秒录音（480000 samples）；退出调试器后完成 10 秒独立录音（160000 samples）。两份录音均无削顶、无启动 200 ms 以外的全零毫秒。
- RTT 中七项 DMA / PCM / USB 错误计数为零。另有正常流启停取消待发包，不能把 `disabled` 或 `discarded_samples` 当作持续运行丢包。
- 98 个每秒跟踪观测中，6 次保护为真、70 次更新为真；累计更新达到 9003 hops。不能据此计算人声识别率。
- 47 个完整录音工作量窗口中，已计入的 DSP + 音频包准备工作量中位数为 51.7%、最高 52.0%。不代表整机 CPU 使用率或严格 WCET。
- 静态 SRAM 29768 bytes，Flash 映像 92568 bytes；相对先前蓝灯版增加 600 bytes /2888 bytes。退出调试器后读取的复位捕获位为清除状态，CFSR/HFSR 为零。
- 45 秒 Windows 轮询中未观察到设备变化。结束时保留实验版运行，调试器已关闭。

源码提交、映像哈希、录音和统计范围见 [首次实板验证记录](../artifacts/adaptive-noise-hardware-validation.json)。原始文件位于本地 `captures/adaptive-live-20261106/`，不入 Git。

## 局限与后续验收

弱语音、气声或无声辅音可能被当成背景；低信噪比谐波用例刻意展示这一局限，不要求其全程冻结才能算其他测试通过。强噪声、新增明显音调可能反而冻结跟踪，仍需手动重校准。

本次只是短时实板试运行，尚无受控风扇变速对照、长期稳定性、栈峰值或主观音质验收；未独立确认用户是否遵循了提示的说话 / 安静时段。独立 10 秒录音期间未读取内部计数。后续应先安静校准，再改变风扇强度，交替说话、停顿、轻声说话；保留原声 / 固定谱 / 自动谱对照，重点听字头、句尾、轻声和音量抽动。通过这些检查后再考虑设为默认。

实现为本项目的独立保守启发式，没有移植 SpeexDSP；可对照阅读 [SpeexDSP 上游噪声估计实现](https://github.com/xiph/speexdsp/blob/master/libspeexdsp/preprocess.c)。
