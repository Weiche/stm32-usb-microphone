"""Offline, calibrated Wiener baseline for mono 16 kHz s16 microphone WAVs.

Requires NumPy and Matplotlib; does not access the microphone or change firmware.
The calibration region must contain background noise without speech. The noise
profile is fixed after calibration; continuous noise tracking is not implemented.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path
import platform
import wave

import numpy as np


RATE = 16000
FFT_SIZE = 256
HOP = 128
WINDOW = np.sqrt(0.5 - 0.5 * np.cos(2 * np.pi * np.arange(FFT_SIZE) / FFT_SIZE))


def read_wav(path):
    with wave.open(str(path), "rb") as source:
        if (source.getnchannels(), source.getsampwidth(), source.getframerate(),
                source.getcomptype()) != (1, 2, RATE, "NONE"):
            raise ValueError("Expected mono 16 kHz signed 16-bit PCM WAV")
        count = source.getnframes()
        raw = source.readframes(count)
    if not count or len(raw) != count * 2:
        raise ValueError("Empty or truncated WAV")
    return np.frombuffer(raw, dtype="<i2").astype(np.float64) / 32768


def write_wav(path, samples):
    if not np.all(np.isfinite(samples)):
        raise ValueError("Non-finite output samples")
    rounded = np.rint(samples * 32768)
    clipped = int(np.count_nonzero((rounded < -32768) | (rounded > 32767)))
    pcm = np.clip(rounded, -32768, 32767).astype("<i2")
    with wave.open(str(path), "wb") as output:
        output.setparams((1, 2, RATE, 0, "NONE", "not compressed"))
        output.writeframes(pcm.tobytes())
    return clipped


def region(samples, interval):
    start, end = interval
    if not (math.isfinite(start) and math.isfinite(end)
            and 0 <= start < end <= len(samples) / RATE):
        raise ValueError(f"Invalid interval {interval} for {len(samples) / RATE:g} s WAV")
    result = samples[round(start * RATE):round(end * RATE)]
    if not len(result):
        raise ValueError("Interval has no samples")
    return result


def power_spectrum(samples):
    if len(samples) < FFT_SIZE:
        raise ValueError("At least one complete FFT frame is required")
    frames = np.lib.stride_tricks.sliding_window_view(samples, FFT_SIZE)[::HOP]
    return np.mean(np.abs(np.fft.rfft(frames * WINDOW, axis=1)) ** 2, axis=0)


def noise_profile(samples, interval):
    reference = region(samples, interval)
    if len(reference) < FFT_SIZE + 7 * HOP:
        raise ValueError("Noise calibration needs at least eight complete frames")
    return np.maximum(power_spectrum(reference), 1e-20)


def denoise(samples, noise_power, max_attenuation_db=12.0):
    """Decision-directed Wiener gains, smoothed across frequency and time.

    Analysis and synthesis use a periodic sqrt-Hann, 50% overlap. Padding and
    overlap normalization preserve both boundaries and the exact sample count.
    The returned file is aligned; this does not measure streaming latency.
    """
    if not math.isfinite(max_attenuation_db) or not 0 <= max_attenuation_db <= 24:
        raise ValueError("Maximum attenuation must be between 0 and 24 dB")
    if samples.ndim != 1 or not len(samples) or not np.all(np.isfinite(samples)):
        raise ValueError("Expected non-empty finite mono samples")
    noise_power = np.asarray(noise_power, dtype=np.float64)
    if (noise_power.shape != (FFT_SIZE // 2 + 1,)
            or not np.all(np.isfinite(noise_power)) or np.any(noise_power < 0)):
        raise ValueError("Invalid noise spectrum")
    noise_power = np.maximum(noise_power, 1e-20)
    floor = 10 ** (-max_attenuation_db / 20)
    padded = np.pad(samples, (FFT_SIZE, FFT_SIZE))
    output = np.zeros_like(padded)
    normalization = np.zeros_like(padded)
    previous_gain = np.full_like(noise_power, floor)
    previous_posterior = np.ones_like(noise_power)
    for start in range(0, len(padded) - FFT_SIZE + 1, HOP):
        spectrum = np.fft.rfft(padded[start:start + FFT_SIZE] * WINDOW)
        posterior = np.minimum(np.abs(spectrum) ** 2 / noise_power, 1e12)
        prior = (0.96 * previous_gain ** 2 * previous_posterior
                 + 0.04 * np.maximum(posterior - 1, 0))
        target = np.clip(prior / (1 + prior), floor, 1)
        neighbors = np.pad(target, (1, 1), mode="edge")
        target = 0.25 * neighbors[:-2] + 0.5 * neighbors[1:-1] + 0.25 * neighbors[2:]
        # Open rapidly for new speech, close more slowly to reduce pumping.
        smoothing = np.where(target > previous_gain, 0.2, 0.8)
        gain = np.clip(smoothing * previous_gain + (1 - smoothing) * target, floor, 1)
        output[start:start + FFT_SIZE] += np.fft.irfft(spectrum * gain, n=FFT_SIZE) * WINDOW
        normalization[start:start + FFT_SIZE] += WINDOW ** 2
        previous_gain = gain
        previous_posterior = posterior
    cut = slice(FFT_SIZE, FFT_SIZE + len(samples))
    if np.any(normalization[cut] < 1e-12):
        raise ValueError("Incomplete overlap-add coverage")
    return output[cut] / normalization[cut]


def rms(samples):
    return float(np.sqrt(np.mean(samples ** 2)))


def db_ratio(numerator, denominator):
    if numerator <= 0 or denominator <= 0:
        return None
    return 20 * math.log10(numerator / denominator)


def segment_metrics(original, processed, interval):
    before = region(original, interval)
    after = region(processed, interval)
    before_rms, after_rms = rms(before), rms(after)
    return {
        "interval_seconds": list(interval),
        "input_rms_lsb": before_rms * 32768,
        "output_rms_lsb": after_rms * 32768,
        "level_reduction_db": db_ratio(before_rms, after_rms),
        "scope": "Total recorded level change; not clean-speech SNR or intelligibility.",
    }


def render_plot(path, original, versions, evaluation):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    colors = ["#64748b", "#0891b2", "#d97706"]
    fig, axes = plt.subplots(2, 1, figsize=(11, 7), constrained_layout=True)
    all_versions = {"Original": original, **versions}
    width = RATE // 10
    times = (np.arange(len(original) // width) + 0.5) * width / RATE
    for (name, samples), color in zip(all_versions.items(), colors):
        frames = samples[:len(times) * width].reshape(-1, width)
        levels = 20 * np.log10(np.maximum(np.sqrt(np.mean(frames ** 2, axis=1)), 1e-12))
        axes[0].plot(times, levels, label=name, color=color, linewidth=1)
        spectrum = power_spectrum(region(samples, evaluation))
        density = spectrum / (RATE * np.sum(WINDOW ** 2))
        density[1:-1] *= 2
        axes[1].plot(np.fft.rfftfreq(FFT_SIZE, 1 / RATE),
                     10 * np.log10(np.maximum(density, 1e-24)), color=color, label=name)
    axes[0].axvspan(*evaluation, color="#cbd5e1", alpha=0.25, label="Held-out background interval")
    axes[0].set(xlabel="Time (s)", ylabel="100 ms RMS (dBFS)",
                title="Same recording and gain; no automatic loudness normalization")
    axes[1].set(xlabel="Frequency (Hz)", ylabel="Power density (dBFS / Hz)",
                title=f"Background spectrum: {evaluation[0]:g}-{evaluation[1]:g} s")
    for axis in axes:
        axis.grid(alpha=0.2)
        axis.legend(loc="best", fontsize=8)
    fig.savefig(path, dpi=150)
    plt.close(fig)


def run(args):
    original = read_wav(args.wav)
    profile = noise_profile(original, args.calibration)
    region(original, args.evaluation)
    region(original, args.activity)
    excerpt = region(original, args.excerpt)
    if max(args.calibration[0], args.evaluation[0]) < min(args.calibration[1], args.evaluation[1]):
        raise ValueError("Background evaluation must not overlap noise calibration")
    versions = {"Mild (6 dB cap)": denoise(original, profile, 6),
                "Stronger (12 dB cap)": denoise(original, profile, 12)}
    playback_gain = 10 ** (args.listen_gain_db / 20)
    all_signals = [original, *versions.values()]
    playback_peak = max(float(np.max(np.abs(x))) for x in all_signals) * playback_gain
    if playback_peak >= 32767 / 32768:
        raise ValueError("Shared listening gain would clip; choose a lower --listen-gain-db")
    # Validate before creating the output directory; old experiments survive.
    output = args.output_dir
    output.mkdir(parents=True, exist_ok=False)
    np.save(output / "noise-power.npy", profile, allow_pickle=False)
    write_wav(output / "original.wav", original)
    write_wav(output / "listen-original.wav", excerpt * playback_gain)
    results = []
    for (name, samples), strength in zip(versions.items(), [6, 12]):
        stem = f"wiener-{strength}db"
        clipped = write_wav(output / f"{stem}.wav", samples)
        # Metrics use the quantized delivered WAV, at the source's original gain.
        delivered = read_wav(output / f"{stem}.wav")
        write_wav(output / f"listen-{stem}.wav", region(delivered, args.excerpt) * playback_gain)
        results.append({"name": name, "max_spectral_attenuation_db": strength,
                        "file": str(output / f"{stem}.wav"), "samples": len(delivered),
                        "clipped_samples_at_write": clipped,
                        "background": segment_metrics(original, delivered, args.evaluation),
                        "speech_requested_region": segment_metrics(original, delivered, args.activity)})
    render_plot(output / "comparison.png", original, versions, args.evaluation)
    report = {
        "input": str(args.wav), "input_sha256": hashlib.sha256(args.wav.read_bytes()).hexdigest(),
        "rate_hz": RATE, "samples": len(original), "duration_seconds": len(original) / RATE,
        "python": platform.python_version(), "numpy": np.__version__,
        "algorithm": "Calibrated decision-directed Wiener baseline; fixed noise PSD, gain smoothing",
        "parameters": {"fft_size": FFT_SIZE, "hop_samples": HOP,
                       "window": "periodic sqrt-Hann, normalized overlap-add",
                       "decision_directed_alpha": 0.96,
                       "gain_smoothing_open_close": [0.2, 0.8],
                       "frequency_smoothing_kernel": [0.25, 0.5, 0.25]},
        "calibration_seconds": list(args.calibration), "results": results,
        "listening_copies": {"excerpt_seconds": list(args.excerpt),
                             "shared_gain_db": args.listen_gain_db,
                             "individual_loudness_normalization": False,
                             "clipping": False},
        "limitations": ["Noise-only and speech regions follow the user's recording protocol; not independently transcribed.",
                        "Real recording has no clean-speech reference. Level reduction is not an SNR improvement measurement.",
                        "Calibration precedes processing the entire file, including its beginning; not a streaming startup implementation.",
                        "Fixed profile does not track changes in the fan/AC after calibration.",
                        "Aligned offline output does not measure MCU CPU load, memory, end-to-end latency or USB stability.",
                        "Audio naturalness and consonant preservation require listening."],
    }
    (output / "report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False,
                                                  allow_nan=False) + "\n", encoding="utf-8")
    print(json.dumps({"output_dir": str(output), "results": results}, indent=2, ensure_ascii=False))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wav", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True, help="New directory; refuses overwrite")
    parser.add_argument("--calibration", type=float, nargs=2, default=(0.5, 4.5), metavar=("START", "END"))
    parser.add_argument("--evaluation", type=float, nargs=2, default=(25, 30), metavar=("START", "END"))
    parser.add_argument("--activity", type=float, nargs=2, default=(5, 15), metavar=("START", "END"))
    parser.add_argument("--excerpt", type=float, nargs=2, default=(3, 15), metavar=("START", "END"))
    parser.add_argument("--listen-gain-db", type=float, default=12)
    args = parser.parse_args()
    if not math.isfinite(args.listen_gain_db) or not -24 <= args.listen_gain_db <= 36:
        parser.error("Listening gain must be finite and between -24 and 36 dB")
    try:
        run(args)
    except (ValueError, FileExistsError) as exc:
        parser.error(str(exc))


if __name__ == "__main__":
    main()
