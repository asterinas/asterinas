# Booting Asterinas on Milk-V Megrez

This document describes how to build a U-Boot-loadable Asterinas image and boot
it on the board.

## 1. Image Format

Asterinas builds to a raw ELF file. U-Boot's `booti` command expects a RISC-V
Linux Image with a 64-byte header:

| Offset | Size | Meaning |
|--------|------|---------|
| 0x00   | 4    | First instruction: `j 0x40` |
| 0x08   | 8    | `text_offset` = 0x200000 |
| 0x10   | 8    | `image_size` |
| 0x38   | 4    | Magic `0x05435352` |

Use [`scripts/mkimage.py`](scripts/mkimage.py) to prepend this header to the raw
binary extracted from the ELF:

```bash
python3 porting/scripts/mkimage.py \
    target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin \
    aster-nix.booti
```

The output `aster-nix.booti` can be loaded at physical address `0x80200000`.

## 2. Copy the Image to the Board

Place the image on the boot partition of the SD card:

```bash
# From Windows/WSL
scp aster-nix.booti anjie@192.168.100.2:/tmp/

# On the board
sudo cp /tmp/aster-nix.booti /boot/aster-nix.booti
```

## 3. U-Boot Commands

Power on, interrupt autoboot, then run:

```bash
ext4load mmc 1:1 0xf0000000 /dtbs/linux-image-6.6.87-win2030/eswin/eic7700-milkv-megrez.dtb
ext4load mmc 1:1 0x80200000 /aster-nix.booti
booti 0x80200000 - 0xf0000000
```

Expected early boot markers from `ostd/src/arch/riscv/boot/boot.S`:

```text
AEFGDHB C...
```

Each letter is emitted by a milestone in the assembly boot code:

| Marker | Meaning |
|--------|---------|
| `A` | Entered `_start`, DTB looks valid |
| `E` | Early trap handler installed |
| `F` | Wrote `0` to `satp` |
| `G` | Read `satp` back successfully |
| `D` | Page table entry for kernel high-half written |
| `H` | Sv48 SATP accepted and read back |
| `h` | Sv39 SATP accepted and read back (fallback) |
| `B` | MMU enabled, jumping to high-half virtual address |
| `C` | Reached virtual-mode Rust entry `riscv_boot` |

## 4. Troubleshooting

### Stops after `AEFGD`

The MMU was enabled but the next milestone (`H`) was never reached. The most
likely cause is a missing `PTE_A` bit on a non-leaf page-table entry. See
[`issues/03-asterinas-mmu-hang-at-AEFGD.md`](issues/03-asterinas-mmu-hang-at-AEFGD.md).

### `Wrong Image Format for bootm command`

This appears if you let autoboot run with the default `bootcmd` (which uses
`bootm`). Interrupt autoboot and use `booti` instead.

### No serial output at all

- Check FTDI attachment in WSL (`ls /dev/ttyUSB*`)
- Verify baud rate is 115200
- If the board is hung, a physical reset is required (break signal / DTR-RTS
  toggling do not reset this board)

## 5. Back to Linux

If Asterinas hangs, power-cycle or press the reset button. U-Boot will either
autoboot the default image or you can re-run the Linux boot commands from
[`setup.md`](setup.md).
