"""Inspect ELF32 load/allocated sections, enforce Flash/SRAM placement, optionally export .bin."""

import argparse
import json
from pathlib import Path
import struct


def inspect(path, binary=None):
    raw = path.read_bytes()
    if raw[:6] != b"\x7fELF\x01\x01":
        raise ValueError("Expected little-endian ELF32")
    header = struct.unpack_from("<16sHHIIIIIHHHHHH", raw)
    phoff, shoff, phsize, phnum, shsize, shnum, strindex = header[5], header[6], header[9], header[10], header[11], header[12], header[13]
    sections = [struct.unpack_from("<IIIIIIIIII", raw, shoff + i * shsize) for i in range(shnum)]
    strings = sections[strindex]
    names = raw[strings[4]:strings[4] + strings[5]]
    allocations = []
    sram = 0
    for section in sections:
        name = names[section[0]:].split(b"\0", 1)[0].decode()
        flags, address, size = section[2], section[3], section[5]
        if not flags & 2 or not size:
            continue
        if 0x20000000 <= address and address + size <= 0x20020000:
            sram += size
            region = "SRAM1/2"
        elif 0x08000000 <= address and address + size <= 0x08100000:
            region = "FLASH"
        else:
            raise ValueError(f"Allocated section outside approved SRAM/Flash: {name} 0x{address:x} size {size}")
        allocations.append({"section": name, "address": f"0x{address:08x}", "bytes": size, "region": region})
    segments = []
    for i in range(phnum):
        kind, offset, _vaddr, paddr, filesz, _memsz, _flags, _align = struct.unpack_from("<IIIIIIII", raw, phoff + i * phsize)
        if kind == 1 and filesz and 0x08000000 <= paddr < 0x08100000:
            if paddr + filesz > 0x08100000:
                raise ValueError("Load segment overflows Flash")
            segments.append((paddr, raw[offset:offset + filesz]))
    if not segments:
        raise ValueError("No Flash load segments")
    flash_end = max(address + len(data) for address, data in segments)
    if binary:
        image = bytearray(b"\xff" * (flash_end - 0x08000000))
        for address, data in segments:
            image[address - 0x08000000:address - 0x08000000 + len(data)] = data
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(image)
    return {"elf": str(path), "entry": f"0x{header[4]:08x}",
            "flash_image_span_bytes": flash_end - 0x08000000,
            "allocated_sram_bytes": sram, "sram_capacity_bytes": 128 * 1024,
            "ccm_allocations": 0, "stack_high_water": "not measured",
            "sections": allocations}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    parser.add_argument("--bin", type=Path)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    report = json.dumps(inspect(args.elf, args.bin), indent=2)
    print(report)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(report + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
