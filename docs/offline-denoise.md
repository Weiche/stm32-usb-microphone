# 离线风扇 / 空调降噪实验

2026-11-05，用户配合从板载 USB 麦克风录制 30 秒原始音频。
本轮只处理保存的 WAV，固件未加入降噪；来源与实测摘要见 [验证记录](../artifacts/denoise-offline-validation.json)。
后续已进行 [板上移植与验证](live-denoise.md)；本文保留离线实验当时的条件与结果。

## 录音与对比条件

录音约定：前 5 秒安静，中间 20 秒正常说话，最后 5 秒安静；风扇、空调及麦克风位置保持不变。
原始录音 `captures/denoise-room-30s.wav` 为 16 kHz、s16le、单声道，480000 samples；实际采集耗时 30.0138435 秒。
约 6～13 秒观察到较高电平的声音活动；语音内容未经转写。噪声 / 说话标签依据录音约定，未作独立声源识别。

校准使用 0.5～4.5 秒，避开启动瞬态；背景评估单独使用 25～30 秒，未参与噪声谱估计。
5～15 秒作为说话请求区间，里面也包含停顿和背景声，不能将这段的总电平变化解释为纯人声损失。

| 输出 | 最大频点幅度衰减参数 | 末尾背景 RMS 降低 | 5～15 秒整体 RMS 降低 |
| --- | --- | --- | --- |
| 轻度 | 6 dB | 5.07 dB | 0.87 dB |
| 较强 | 12 dB | 9.54 dB | 1.13 dB |

用户试听反馈：目前看来较强档没有明显问题，因此选定较强档（最大频点衰减 12 dB）作为后续 MCU 移植基线。
该结论对应本次录音和试听条件；尚未完成板上降噪实现及 CPU、延迟、稳定性验证。

原始背景 RMS 213.38 LSB；轻度输出 118.99 LSB，较强输出 71.14 LSB。
这表示本段背景电平降低，不是有干净人声参考的 SNR 改善，也不证明辅音、句尾和自然度无损。
两档输出均保留全部 480000 samples，量化写出时削顶样本为 0；原声复制文件的 PCM 与输入完全一致。

`captures/denoise-offline-v1/` 中保存完整原声、两档完整输出、噪声谱、图和报告。
`listen-*.wav` 为同一段 3～15 秒片段，全部使用统一 +12 dB 播放增益，均无削顶；未分别作响度归一化。
可先听字头、轻声及句尾，再听停顿时是否有金属音、忽大忽小或其他伪影。
原始音频和试听文件在本地 `captures/`，不纳入 Git；克隆仓库只能获得代码与摘要。

## 算法范围

`tools/denoise_offline.py` 是独立实现的校准 Wiener 基线，不是 SpeexDSP 的移植。
使用 256 点实 FFT、128 samples 步进、周期 sqrt-Hann 分析 / 合成窗和归一化重叠相加。
对应 16 ms 窗长、8 ms 更新间隔；文件通过补零和裁剪对齐原始采样，端点不会直接被窗口抹掉。

先从已知安静区间求平均噪声功率谱 `N[k]`，之后保持固定。
每帧后验 SNR 为 `gamma = |Y|²/N`；决策导向先验为
`xi = 0.96 * previous_gain² * previous_gamma + 0.04 * max(gamma - 1, 0)`。
Wiener 幅度增益为 `xi/(1+xi)`，限制到 `[10^(-cap_dB/20), 1]`。
再使用 `[0.25, 0.5, 0.25]` 跨频点平滑，以及增益上升 / 下降系数 0.2 / 0.8 的时间平滑。

此次未叠加额外高通、噪声门或 AGC，以便观察这一项降噪本身的作用。
当前离线版本先读校准段再处理整段，包含校准段之前的样本；它不是实时启动流程。
没有自动 VAD 或动态噪声谱更新，风扇转速或空调状态变化后需要重新校准。
NumPy 计算使用桌面浮点实现；MCU 的 f32 / 定点数值误差、RAM、CPU 占用、实时调度和端到端延迟均未测量。

FFT / 重叠相加依据可查 [NumPy 实 FFT 文档](https://numpy.org/doc/stable/reference/generated/numpy.fft.rfft.html) 和 [SciPy NOLA 说明](https://docs.scipy.org/doc/scipy/reference/generated/scipy.signal.check_NOLA.html)。
成熟语音预处理器的后续比较对象可参考 [SpeexDSP 上游实现](https://github.com/xiph/speexdsp/blob/master/libspeexdsp/preprocess.c)，本轮未做该比较。

## 复现与检查

```powershell
python -m pip install -r tools/requirements-denoise.txt
python -m unittest discover -s tools -p test_denoise_offline.py -v
python tools/denoise_offline.py captures/denoise-room-30s.wav --output-dir captures/denoise-offline-v2
```

输出目录必须不存在，避免覆盖已有实验。其他录音可通过 `--calibration START END`、`--evaluation START END`、`--activity START END` 和 `--excerpt START END` 指定区间；背景评估不得与校准区间重叠。
试听增益默认统一 +12 dB，若会削顶则拒绝生成，可用 `--listen-gain-db` 降低。

5 项信号级检查覆盖：旁路时首尾与非整帧长度重建、静音数值安全、强音保留、独立校准噪声与已知合成信号的重建误差改善、非法区间 / 频谱拒绝。
已知信号测试使用调幅多谐波合成信号，不是真人语音；它不能代替本次录音的主观试听。
