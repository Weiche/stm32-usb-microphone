"""Exercise the exact Rust DSP with deterministic noise changes and optional room audio.

Outputs local WAV/CSV files and an inspectable JSON report. Synthetic harmonics
are not real speech; the optional room recording has no clean-speech reference.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

import numpy as np

from denoise_offline import HOP, RATE, noise_profile, read_wav, rms, write_wav


ROOT = Path(__file__).resolve().parents[1]
HOST = "x86_64-pc-windows-msvc"


def run_dsp(executable, directory, samples, profile, adaptive):
    prefix = "adaptive" if adaptive else "fixed"
    raw = directory / "input.s16le"
    noise = directory / "noise.f32le"
    output = directory / f"{prefix}.s16le"
    trace = directory / f"{prefix}.csv"
    raw.write_bytes(np.clip(np.rint(samples * 32768), -32768, 32767).astype("<i2").tobytes())
    noise.write_bytes(np.asarray(profile, dtype="<f4").tobytes())
    command = [str(executable), str(raw), str(output), str(noise), "--trace", str(trace)]
    if adaptive:
        command.append("--adaptive")
    subprocess.run(command, check=True)
    actual = np.frombuffer(output.read_bytes(), dtype="<i2").astype(np.float64) / 32768
    if len(actual) != len(samples) or not np.all(np.isfinite(actual)):
        raise ValueError("Invalid Rust output")
    write_wav(directory / f"{prefix}.wav", actual)
    telemetry = np.genfromtxt(trace, delimiter=",", names=True)
    # The host wrapper flushes the fixed delay; exclude that padding from metrics.
    telemetry = telemetry[telemetry["input_samples"] <= len(samples)]
    return actual, telemetry


def interval_trace(trace, start, end):
    return trace[(trace["input_samples"] >= start * RATE)
                 & (trace["input_samples"] < end * RATE)]


def db(a, b):
    return float(20 * np.log10(max(a, 1e-20) / max(b, 1e-20)))


def validate(output_dir, room_wav=None):
    output_dir.mkdir(parents=True, exist_ok=False)
    subprocess.run(["cargo", "build", "--locked", "--release", "--example", "denoise_pcm",
                    "--target", HOST], cwd=ROOT, check=True)
    executable = ROOT / "target" / HOST / "release" / "examples" / "denoise_pcm.exe"
    rng = np.random.default_rng(20261106)
    sigma = 0.01
    calibration = rng.normal(0, sigma, RATE * 5)
    profile = noise_profile(calibration, (0.5, 4.5))
    cases = []
    for name, scale in (("noise-rise", 1.5), ("noise-fall", 0.65)):
        noise = rng.normal(0, sigma, RATE * 40)
        noise[5 * RATE:] *= scale
        cases.append((name, noise, profile))
    t = np.arange(RATE * 30) / RATE
    # Continuous voiced-like harmonics with a syllabic envelope; known reference.
    voice = sum(0.045 / h * np.sin(2 * np.pi * 187 * h * t) for h in range(1, 16))
    voice *= 0.5 + 0.5 * np.sin(2 * np.pi * 2.7 * t) ** 2
    voice[:5 * RATE] = 0
    voice[25 * RATE:] = 0
    cases.append(("harmonics", voice + rng.normal(0, sigma, len(t)), profile))
    cases.append(("weak-harmonics", voice * 0.04 + rng.normal(0, sigma, len(t)), profile))
    cases.append(("silent-calibration", voice, np.zeros_like(profile)))
    if room_wav:
        cases.append(("room-recording", read_wav(room_wav),
                      noise_profile(read_wav(room_wav), (0.5, 4.5))))
    results = {}
    checks = {}
    for name, samples, initial in cases:
        directory = output_dir / name
        directory.mkdir()
        write_wav(directory / "input.wav", samples)
        fixed, _ = run_dsp(executable, directory, samples, initial, False)
        adaptive, trace = run_dsp(executable, directory, samples, initial, True)
        result = {
            "samples": len(samples),
            "input_pcm_sha256": hashlib.sha256((directory / "input.s16le").read_bytes()).hexdigest(),
            "final_power_ratio": float(trace["power_permille"][-1] / 1000),
            "update_hops": int(trace["updates"][-1]),
            "guarded_hops_fraction": float(np.mean(trace["speech_guard"])),
            "full_scale_output_samples": int(np.count_nonzero((adaptive <= -1) | (adaptive >= 32767 / 32768))),
        }
        if name.startswith("noise-"):
            tail = slice(-5 * RATE, None)
            extra = db(rms(fixed[tail]), rms(adaptive[tail]))
            result["last_5s_extra_attenuation_vs_fixed_db"] = extra
            result["last_5s_input_to_output_attenuation_db"] = db(rms(samples[tail]), rms(adaptive[tail]))
            if name == "noise-rise":
                checks["moderate_rise_followed"] = 1.9 < result["final_power_ratio"] < 2.6
                checks["rise_residual_reduced"] = extra > 0.5
            else:
                checks["moderate_fall_followed"] = 0.35 < result["final_power_ratio"] < 0.55
        elif name in ("harmonics", "weak-harmonics", "silent-calibration"):
            active = interval_trace(trace, 5.1, 25)
            result["known_signal_guard_fraction"] = float(np.mean(active["speech_guard"]))
            result["known_signal_update_hops"] = int(np.count_nonzero(active["updating"]))
            if name != "weak-harmonics":
                checks[f"{name}_not_learned"] = result["known_signal_update_hops"] == 0
            else:
                checks["weak_harmonics_learning_limited"] = result["known_signal_update_hops"] < 0.05 * len(active)
                result["scope"] = "Low-SNR synthetic limitation probe. Limited learning is required; this is not real speech VAD accuracy."
            if name == "harmonics":
                selection = slice(6 * RATE, 24 * RATE)
                fixed_error = rms(fixed[selection] - voice[selection])
                adaptive_error = rms(adaptive[selection] - voice[selection])
                result["known_signal_error_change_db"] = db(adaptive_error, fixed_error)
                checks["harmonic_reconstruction_not_degraded"] = result["known_signal_error_change_db"] < 0.5
        else:
            # Recording-protocol labels; pauses are included, no claimed VAD accuracy.
            for label, a, b in (("activity_requested", 5, 15), ("background_requested", 25, 30)):
                if len(samples) < b * RATE:
                    continue
                frames = interval_trace(trace, a, b)
                selection = slice(a * RATE, b * RATE)
                result[label] = {
                    "interval_seconds": [a, b],
                    "guarded_fraction": float(np.mean(frames["speech_guard"])),
                    "updating_fraction": float(np.mean(frames["updating"])),
                    "adaptive_vs_fixed_level_db": db(rms(adaptive[selection]), rms(fixed[selection])),
                }
            result["source_sha256"] = hashlib.sha256(room_wav.read_bytes()).hexdigest()
        checks[f"{name}_no_output_saturation"] = result["full_scale_output_samples"] == 0
        results[name] = result
    report = {
        "algorithm": "Anchored energy and slow normalized spectral guard, 504ms quiet hold, bounded PSD tracking; exact firmware Rust DSP on host",
        "parameters": {"hop_samples": HOP, "update_alpha": 0.0016,
                       "per_bin_power_bounds_vs_calibration": [0.25, 4.0],
                       "weak_spectral_alpha": 0.02, "weak_spectral_excess_ratio": 1.5,
                       "weak_spectral_votes": 8, "weak_band_hz": [125, 4000]},
        "source_sha256": {path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest()
                          for path in ("src/noise_tracker.rs", "src/denoiser.rs", "examples/denoise_pcm.rs",
                                       "tools/validate_adaptive_noise.py")},
        "host_executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
        "git_base": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "checks": checks,
        "passed": all(checks.values()),
        "results": results,
        "limitations": [
            "No on-board CPU, USB, timing, or stack verification of this new firmware.",
            "Harmonics are synthetic, not a speech intelligibility benchmark.",
            "Room segments follow the old recording protocol, contain pauses and have no clean-speech reference.",
            "Energy guard can confuse weak speech and noise; strong or tonal background changes may freeze tracking.",
            "Noise-change scenarios are deterministic synthetic signals, not a recorded fan-speed change.",
            "The host uses a precomputed noise profile; startup calibration is tested separately in Rust.",
            "Listening is required before enabling by default.",
        ],
    }
    (output_dir / "report.json").write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True, help="New local directory; refuses overwrite")
    parser.add_argument("--room-wav", type=Path, help="Optional original 30s recording following the old quiet/speech/quiet protocol")
    parser.add_argument("--report", type=Path, help="Optional additional JSON evidence path")
    args = parser.parse_args()
    report = validate(args.output_dir, args.room_wav)
    if args.report:
        args.report.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, allow_nan=False))
    if not report["passed"]:
        raise SystemExit("Adaptive noise validation failed")


if __name__ == "__main__":
    main()
