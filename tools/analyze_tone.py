"""Check a stage-1 16 kHz, signed 16-bit mono WAV without third-party packages."""

import argparse
from array import array
import json
import math
from pathlib import Path
import sys
import wave


def analyze(path):
    with wave.open(str(path), "rb") as wav:
        if (wav.getnchannels(), wav.getsampwidth(), wav.getframerate(), wav.getcomptype()) != (1, 2, 16000, "NONE"):
            raise ValueError("Expected uncompressed 16 kHz / 16-bit / mono WAV")
        samples = array("h", wav.readframes(wav.getnframes()))
    if sys.byteorder != "little":
        samples.byteswap()
    if len(samples) < 16000:
        raise ValueError("Need at least one second of samples")
    # Skip startup, measure a one-second, integer-cycle rectangular window.
    start = min(16000, len(samples) - 16000)
    window = samples[start:start + 16000]
    dc = sum(window) / len(window)
    rms = math.sqrt(sum((x - dc) ** 2 for x in window) / len(window))
    peak = max(abs(x) for x in window)

    def power(freq):
        coefficient = 2 * math.cos(2 * math.pi * freq / 16000)
        a = b = 0.0
        for value in window:
            a, b = value - dc + coefficient * a - b, a
        return a * a + b * b - coefficient * a * b

    powers = {f: power(f) for f in range(950, 1051)}
    frequency = max(powers, key=powers.get)
    fundamental_power = powers[frequency]
    harmonics = sum(power(frequency * n) for n in range(2, 8))
    thd = math.sqrt(max(0, harmonics / fundamental_power)) if fundamental_power else math.inf
    # A repeated 16-sample period should be identical once the Windows path settles.
    # This diagnoses corruption; zero does NOT prove absence of whole-period USB losses.
    periodic_mismatches = sum(samples[i] != samples[i - 16] for i in range(start + 16, len(samples)))
    silent_ms = sum(not any(samples[i:i + 16]) for i in range(start, len(samples) - 15, 16))
    passed = (abs(frequency - 1000) <= 1 and abs(dc) < 100 and
              7000 <= peak <= 9500 and 4500 <= rms <= 7000 and thd < 0.01 and silent_ms == 0)
    return {"file": str(path), "duration_seconds": len(samples) / 16000,
            "frequency_hz_1hz_resolution": frequency, "dc_lsb": dc,
            "peak_lsb": peak, "rms_lsb": rms,
            "peak_dbfs": 20 * math.log10(peak / 32768) if peak else None,
            "rms_dbfs": 20 * math.log10(rms / 32768) if rms else None,
            "thd_ratio_harmonics_2_to_7": thd,
            "clipped_samples": sum(x in (-32768, 32767) for x in samples),
            "period16_mismatches_after_startup": periodic_mismatches,
            "silent_ms_after_startup": silent_ms, "stage1_tone_check_pass": passed,
            "limits": "Windows may resample/process audio. Tone checks cannot prove USB delivery or long-term stability."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wav", type=Path)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    report = analyze(args.wav)
    serialized = json.dumps(report, indent=2, ensure_ascii=False)
    print(serialized)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(serialized + "\n", encoding="utf-8")
    if not report["stage1_tone_check_pass"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
