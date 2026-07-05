#!/usr/bin/env python3
"""Convert an Asterinas RISC-V ELF into a U-Boot booti-compatible image.

The output is a raw binary with a 64-byte RISC-V Linux Image header prepended.
U-Boot's `booti` command can load it at 0x80200000.

Usage:
    python3 mkimage.py <input-elf> <output.booti>

Example:
    python3 mkimage.py \
        target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin \
        aster-nix.booti
"""

import argparse
import struct
import subprocess
import tempfile
from pathlib import Path


def make_booti_image(elf_path: str | Path, out_path: str | Path) -> None:
    elf_path = Path(elf_path)
    out_path = Path(out_path)

    with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as tmp:
        tmp_bin = Path(tmp.name)

    try:
        subprocess.run(
            ["riscv64-linux-gnu-objcopy", "-O", "binary", str(elf_path), str(tmp_bin)],
            check=True,
        )
        raw = tmp_bin.read_bytes()
    finally:
        tmp_bin.unlink(missing_ok=True)

    image_size = len(raw)
    header = bytearray(64)
    struct.pack_into("<I", header, 0, 0x0400006F)   # j 0x40
    struct.pack_into("<I", header, 4, 0)
    struct.pack_into("<Q", header, 8, 0x200000)     # text_offset
    struct.pack_into("<Q", header, 16, image_size)  # image_size
    struct.pack_into("<Q", header, 24, 0)            # flags
    struct.pack_into("<I", header, 32, 0)            # version
    struct.pack_into("<I", header, 36, 0)            # res1
    struct.pack_into("<Q", header, 40, 0)            # res2
    struct.pack_into("<Q", header, 48, 0)            # res3
    struct.pack_into("<I", header, 56, 0x05435352)  # magic
    struct.pack_into("<I", header, 60, 0)            # res4

    out_path.write_bytes(bytes(header) + raw)
    print(f"Created {out_path}, size={len(bytes(header)+raw)}, image_size={image_size}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Create a RISC-V booti image from an Asterinas ELF")
    parser.add_argument("elf", help="Input ELF file")
    parser.add_argument("out", help="Output booti image")
    args = parser.parse_args()
    make_booti_image(args.elf, args.out)


if __name__ == "__main__":
    main()
