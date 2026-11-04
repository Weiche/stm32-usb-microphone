"""Reconstruct the finite PDM snapshot from RTT; reject missing or reordered blocks."""
import argparse
from array import array
import json
from pathlib import Path
import re
import sys


def extract(log):
    words = []
    for match in re.finditer(r"pdm_raw index=(\d+) data=\[([^\]]*)\]", log):
        index = int(match[1])
        if index != len(words):
            raise ValueError(f"Missing/reordered RTT block: wanted {len(words)}, got {index}")
        chunk = [int(token.strip(), 0) for token in match[2].split(",") if token.strip()]
        if len(chunk) != 64 or any(not 0 <= word <= 65535 for word in chunk):
            raise ValueError("Expected exactly 64 u16 words in a snapshot block")
        words.extend(chunk)
    if len(words) != 4096 or "pdm_snapshot end words=4096" not in log:
        raise ValueError(f"Incomplete snapshot ({len(words)} words); keep RTT reader attached")
    return words


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--json", type=Path, required=True)
    args = parser.parse_args()
    log = args.log.read_text(encoding="utf-8-sig")
    words = extract(log)
    ones = sum(word.bit_count() for word in words)
    density = ones / (len(words) * 16)
    bits = [(word >> bit) & 1 for word in words for bit in range(15, -1, -1)]
    transitions = sum(a != b for a, b in zip(bits, bits[1:]))
    stats = re.findall(r"pdm words=(\d+) ones=(\d+) bits=(\d+) pending_words=(\d+) low=(\d+) high=(\d+) ring_overruns=(\d+) dma_errors=(\d+) spi_sr=([0-9a-fx]+)", log)
    if not stats:
        raise ValueError("No capture diagnostics in RTT log")
    latest = stats[-1]
    report = {"word_encoding": "u16 little endian; serial bits MSB first within each word",
              "words": len(words), "bits": len(bits), "nominal_bit_rate_hz": 1024000,
              "nominal_duration_seconds": len(bits)/1024000,
              "ones": ones, "one_density": density, "transitions": transitions,
              "unique_words": len(set(words)), "last_total_words": int(latest[0]),
              "last_interval_bits": int(latest[2]), "dma_water_low_words": int(latest[4]),
              "dma_water_high_words": int(latest[5]), "ring_overruns": int(latest[6]),
              "dma_hardware_errors": int(latest[7]), "spi_status_hex": latest[8],
              "raw_sanity_pass": 0.35 < density < 0.65 and len(set(words)) > 32 and int(latest[6]) == 0 and int(latest[7]) == 0,
              "limits": "Bit density/activity validate a live candidate PDM stream; acoustic fidelity and physical clock accuracy remain separate tests."}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    packed = array("H", words)
    if sys.byteorder != "little": packed.byteswap()
    args.output.write_bytes(packed.tobytes())
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    if not report["raw_sanity_pass"]: raise SystemExit(1)


if __name__ == "__main__":
    main()
