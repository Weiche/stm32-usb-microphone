"""Record the named USB microphone through Windows winmm (Python stdlib only)."""

import argparse
import ctypes as c
from ctypes import wintypes as w
import json
from pathlib import Path
import time
import wave


class Caps(c.Structure):
    _fields_ = [("mid", w.WORD), ("pid", w.WORD), ("version", w.DWORD),
                ("name", w.WCHAR * 32), ("formats", w.DWORD),
                ("channels", w.WORD), ("reserved", w.WORD)]


class Format(c.Structure):
    _pack_ = 1
    _fields_ = [("tag", w.WORD), ("channels", w.WORD), ("rate", w.DWORD),
                ("byte_rate", w.DWORD), ("align", w.WORD),
                ("bits", w.WORD), ("extra", w.WORD)]


class Header(c.Structure):
    _fields_ = [("data", c.c_void_p), ("length", w.DWORD), ("recorded", w.DWORD),
                ("user", c.c_size_t), ("flags", w.DWORD), ("loops", w.DWORD),
                ("next", c.c_void_p), ("reserved", c.c_size_t)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--device", default="STM32F4", help="Unique case-insensitive name substring")
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--output", type=Path, default=Path("captures/stage1.wav"))
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error("--seconds must be positive")
    if not hasattr(c, "WinDLL"):
        parser.error("This recorder requires native Windows Python")
    mm = c.WinDLL("winmm")
    mm.waveInGetNumDevs.restype = w.UINT
    mm.waveInGetDevCapsW.argtypes = [c.c_size_t, c.POINTER(Caps), w.UINT]
    mm.waveInOpen.argtypes = [c.POINTER(w.HANDLE), w.UINT, c.POINTER(Format), c.c_size_t, c.c_size_t, w.DWORD]
    for name in ["waveInPrepareHeader", "waveInUnprepareHeader", "waveInAddBuffer"]:
        getattr(mm, name).argtypes = [w.HANDLE, c.POINTER(Header), w.UINT]
    for name in ["waveInStart", "waveInStop", "waveInReset", "waveInClose"]:
        getattr(mm, name).argtypes = [w.HANDLE]

    def check(result, operation):
        if result:
            raise RuntimeError(f"{operation}: winmm error {result}")

    devices = []
    for index in range(mm.waveInGetNumDevs()):
        caps = Caps()
        check(mm.waveInGetDevCapsW(index, c.byref(caps), c.sizeof(caps)), "get caps")
        devices.append({"id": index, "name": caps.name, "channels": caps.channels})
    print(json.dumps(devices, ensure_ascii=False, indent=2))
    if args.list:
        return
    matches = [d for d in devices if args.device.casefold() in d["name"].casefold()]
    if len(matches) != 1:
        parser.error("Expected exactly one matching USB device; use --list / --device")
    chosen = matches[0]
    handle = w.HANDLE()
    fmt = Format(1, 1, 16000, 32000, 2, 16, 0)
    check(mm.waveInOpen(c.byref(handle), chosen["id"], c.byref(fmt), 0, 0, 0), "open 16k mono s16")
    # Eight 100 ms host buffers, continually requeued. MCU heap constraints do not apply here.
    buffers = [c.create_string_buffer(3200) for _ in range(8)]
    headers = [Header(c.addressof(buf), len(buf), 0, 0, 0, 0, None, 0) for buf in buffers]
    prepared = []
    target_bytes = int(args.seconds * 16000) * 2
    captured = 0
    started = time.monotonic()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        for header in headers:
            check(mm.waveInPrepareHeader(handle, c.byref(header), c.sizeof(header)), "prepare")
            prepared.append(header)
            check(mm.waveInAddBuffer(handle, c.byref(header), c.sizeof(header)), "queue")
        with wave.open(str(args.output), "wb") as output:
            output.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
            check(mm.waveInStart(handle), "start")
            index = 0
            while captured < target_bytes:
                header = headers[index]
                if header.flags & 1:  # WHDR_DONE, consume strictly in queue order
                    count = min(header.recorded, target_bytes - captured)
                    output.writeframesraw(c.string_at(header.data, count))
                    captured += count
                    check(mm.waveInAddBuffer(handle, c.byref(header), c.sizeof(header)), "requeue")
                    index = (index + 1) % len(headers)
                else:
                    time.sleep(0.005)
                if time.monotonic() - started > args.seconds + 15:
                    raise TimeoutError("Capture stalled; inspect USB/RTT and Windows microphone permission")
    finally:
        mm.waveInStop(handle)
        mm.waveInReset(handle)
        for header in prepared:
            mm.waveInUnprepareHeader(handle, c.byref(header), c.sizeof(header))
        mm.waveInClose(handle)
    report = {"device": chosen, "bytes": captured, "samples": captured // 2,
              "sample_rate": 16000, "channels": 1, "bits": 16,
              "wall_seconds": time.monotonic() - started, "file": str(args.output)}
    args.output.with_suffix(".capture.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
