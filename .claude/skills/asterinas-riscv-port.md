---
name: asterinas-riscv-port
description: Expert knowledge for porting Asterinas OS to RISC-V (Milk-V Megrez / EIC7700). Covers boot flow, MMU setup, QEMU-vs-hardware differences, known bugs, diagnostic markers, build workflows, and the full 49-commit history of fixes.
---

# Asterinas RISC-V Port Skill

You are an expert on the Asterinas RISC-V port effort. This skill loads all
accumulated context so you can help effectively without re-deriving everything.

## When to Use This Skill

Use this skill whenever the user asks about:
- Building, testing, or debugging the RISC-V port of Asterinas
- Understanding boot markers, trap handler output, or page table issues
- Deploying to real hardware (Milk-V Megrez / ESWIN EIC7700)
- QEMU compatibility with RISC-V
- Git history management for RISC-V commits
- Any file under `ostd/src/arch/riscv/` or `porting/`

## 1. PROJECT OVERVIEW

**Goal**: Boot Asterinas on the Milk-V Megrez board (ESWIN EIC7700 SoC, SiFive P550 cores).

**Current Status**:
- ✅ QEMU: full boot to panic handler (`AEFGDJBWXV`, 0 page faults) in Sv39 mode
- 🔄 QEMU Sv48: fixing FRAME_METADATA page table issues (Td page faults)
- ❌ Real hardware: first test reached `AEFGD` then silent hang; next test pending

**Git state**: `main` branch, 49 commits ahead of `origin/main` @ `30e061b85`.

## 2. KEY FILES

| File | Role |
|------|------|
| `ostd/src/arch/riscv/boot/boot.S` | Assembly boot entry (560 lines). Page tables, MMU enable, trap handler, diagnostic markers. |
| `ostd/src/arch/riscv/boot/mod.rs` | Rust boot entry. DTB parsing, early info, `KERNEL_PT_READY` static. |
| `ostd/src/arch/riscv/mm/mod.rs` | `PagingConsts`, `PageTableFlags`, `PageTableEntry`. Sv48 (NR_LEVELS=4, ADDRESS_WIDTH=48). |
| `ostd/src/mm/frame/meta.rs` | Frame metadata initialization. `meta::init()` — current crash point. |
| `ostd/src/mm/frame/allocator.rs` | Physical frame allocator. `early_alloc` during boot. |
| `ostd/src/mm/kspace/mod.rs` | Kernel page table init. `init_kernel_page_table()`. |
| `ostd/src/lib.rs` | Main init chain. |
| `porting/` | Documentation, scripts, hardware artifacts, images, issues. |
| `porting/debugging-strategy.md` | Systematic debugging plan (highest-value doc). |
| `porting/retrospective-analysis.md` | Full analysis of 40 commits and upstream diff. |
| `PLAN_Td_FIX.md` | Root cause analysis for QEMU Sv48 Td page faults. |

## 3. BOOT FLOW & MARKER REFERENCE

The boot assembly emits single-character diagnostic markers via SBI putchar.
**Every character means a milestone was reached.** Silence after a marker
means the NEXT milestone crashed.

```
_start:
  A — DTB validated, entered _start
  E — Early trap handler installed (stvec written)
  F — satp written to 0
  G — satp read-back successful
  (page tables built here)
  D — Page table entries written (L4 entries + meta fill complete)
  J — sfence.vma done, about to emit B
  B — Timer disabled, about to enable MMU
  (MMU enabled, satp written with Sv48 + boot_l4pt PPN)
  W — MMU on, SBI still reachable
  X — VMA remap done (stvec, sp, gp moved to high-half)
  V — About to jump to virtual-mode riscv_boot
  C — Reached Rust entry (riscv_boot, MMIO UART write)
  R — DTB parsed successfully via Fdt::from_ptr
  (call_ostd_main → init chain)
  a,b,c,d,e,f,g,h — meta::init() internal markers
  i — frame allocator::init() called
  5,6,7,8 — frame allocator::init() internal stages
  j,1 — kspace::init_kernel_page_table() internal stages
```

**Trap handler markers**:
```
  T — Unexpected trap occurred (not timer)
  [hex nibble] — scause low 4 bits (e.g., Td = Load Page Fault, 0x0d)
  : — separator
  [hex nibble] — stval[47:44], i.e., VPN[3] of faulting VA (L4 index)
```

**Other special markers**:
```
  @ — DTB validation failed (hang)
  ! — SATP write-back verification failed (hang)
```

### scause quick reference

| scause | Meaning |
|--------|---------|
| 0x02 | Illegal instruction |
| 0x0c | Instruction page fault |
| 0x0d | Load page fault (Td in marker notation) |
| 0x0f | Store page fault |
| 0x8000000000000005 | Supervisor timer interrupt |

## 4. Sv48 PAGE TABLE LAYOUT

```
Virtual Address Space:
  0x00000000_00000000 → IDENTITY    L4[0]   → boot_idpt
  0xffff8000_00000000 → LINEAR      L4[256] → boot_idpt (shared)
  0xffffe000_00000000 → FRAME_META  L4[448] → boot_meta_l3pt
  0xffffffff_80000000 → KERNEL      L4[511] → boot_l3pt

boot_idpt:  512 × 1 GiB gigapage leaves (PA 0..512 GiB)
boot_l3pt:  512 × 1 GiB identity leaves for kernel high-half
  L3[510] → boot_l2_kernel (2 MiB leaves for VA 0xffffffff80000000-0xbfffffff)
boot_meta_l3pt: L3[0] → boot_meta_l2
boot_meta_l2:  80 entries → boot_meta_l1_0..79 (each covers 2 MiB of meta space)
boot_meta_l1_*: 64 tables, each 512 × 4 KiB leaf slots (zero-initialized, filled by map_base_page)
```

**PTE bit definitions used in boot.S:**
```
PTE_VRXWAD = V | R | W | X | A | D  (0xcf) — for leaf entries
PTE_VRXW   = V | R | W | X | A | D  (same value, alias)
PTE_V      = 0x01 — for non-leaf pointers
```

## 5. BUILD & TEST WORKFLOWS

### Build for QEMU
```bash
export OSDK_LOCAL_DEV=1
cargo osdk build --scheme riscv --target-arch riscv64
# Output: target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin
```

### QEMU test with diagnostic markers
```bash
./porting/scripts/qemu_run_megrez.sh 45
# Temporarily patches UART base 0x50900000 → 0x10000000
# Output logged to /tmp/aster-qemu-run.log
```

### Build for real board
```bash
export OSDK_LOCAL_DEV=1
cargo osdk build --scheme milkv-megrez --target-arch riscv64
```

### Create booti image
```bash
python3 porting/scripts/mkimage.py \
    target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin \
    aster-nix.board.booti
```

### Copy to board (from WSL)
```bash
/mnt/c/Windows/System32/OpenSSH/scp.exe aster-nix.board.booti anjie@192.168.100.2:/tmp/
```

### On the board (via serial or SSH)
```bash
sudo cp /tmp/aster-nix.board.booti /boot/
# Then reboot, interrupt U-Boot autoboot:
ext4load mmc 1:1 0xf0000000 /dtbs/linux-image-6.6.87-win2030/eswin/eic7700-milkv-megrez.dtb
ext4load mmc 1:1 0x80200000 /aster-nix.board.booti
booti 0x80200000 - 0xf0000000
```

### Automated board test (Python)
```bash
python3 porting/scripts/serial_boot_asterinas.py /dev/ttyUSB0 115200
```

### QEMU with MMU trace (powerful diagnostic)
```bash
qemu-system-riscv64 -machine virt -cpu rv64,zba=true,zbb=true -m 4G \
    -nographic -bios default \
    -dtb porting/images/eic7700-milkv-megrez.dtb \
    -kernel target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin \
    -d mmu,cpu,int -D /tmp/qemu-mmu-trace.log
```

## 6. CATALOG OF KNOWN BUGS (all fixed)

| # | Bug | Symptom | Root Cause | Fix |
|---|-----|---------|------------|-----|
| 1 | SBI ecall clobbers DTB pointer | DTB parse fails | `a1` destroyed by sbiret return struct | Save a1 to s2 BEFORE any ecall |
| 2 | Signed constant sign-extension | Wrong addresses | RISC-V immediate sign-extension semantics | Use explicit sign-extension sequence |
| 3 | SBI_SET_TIMER clobbers t0 | Page table corruption | `rdtime` inside macro overwrites caller's t0 | Document clobbers; reload t0 after |
| 4 | SBI reset wrong type | Board doesn't reset | Wrong reset_type parameter | Use reset_type=1 (cold reboot) |
| 5 | `sbi-rt` crate binary incompatibility | SBI ecall infinite loop | OpenSBI 1.5.1 version check mismatch | Replace all sbi-rt with legacy inline asm |
| 6 | QEMU 8.2.2 Sv39 1 GiB gigapage | Page fault after MMU on | Gigapage is OPTIONAL in Sv39 spec | Use 2 MiB leaves for kernel VMA |
| 7 | QEMU 9.2.4/10.0.0 Sv48 TLB walker | MMU crashes | QEMU TLB walker bug (not kernel) | Avoid Sv48 on affected QEMU versions |
| 8 | FPU not enabled | Illegal instruction on float ops | `FS::Off` depends on unimplemented lazy FPU traps | Use `FS::Initial` |
| 9 | `_Unwind_Backtrace` crashes | Double panic on riscv64 | Unwind triggers illegal instruction on OpenSBI 1.5.1 | Skip unwind on riscv64 |
| 10 | `fill_early_info` called too late | memory_regions read before init | T5 destroyed; regions accessed before EARLY_INFO filled | Call fill_early_info earlier in init chain |
| 11 | Sv39 boot.S + Sv48 PagingConsts | PTE index mismatch | NR_LEVELS=4 but boot page table was Sv39 | Match boot.S page table mode to PagingConsts |
| 12 | Non-leaf PTE missing A bit | Hang after MMU on (AEFGD silence) | Some HW walkers require PTE_A on non-leaf entries | Set PTE_A on non-leaf PTEs |
| 13 | L4[448] missing for FRAME_META | Td (Load Page Fault) in meta::init() | FRAME_METADATA_BASE_VADDR VPN[3]=448 had no L4 entry | Add L4[448] → boot_meta_l3pt |
| 14 | LINEAR_MAPPING only 4 GiB | Td accessing PA > 4 GiB | boot_idpt had only 4 × 2 MiB entries | 512 × 1 GiB gigapages covering 0..512 GiB |

## 7. QEMU vs REAL HARDWARE DIFFERENCES

| Item | QEMU `virt` | Milk-V Megrez | Impact |
|------|-------------|---------------|--------|
| UART base | `0x10000000` (NS16550) | `0x50900000` (dw-apb-uart) | Early boot uses SBI putchar (safe); later MMIO writes differ |
| DTB source | QEMU generated | Real `/boot` DTB | Memory layout, CPU topology, reserved regions differ |
| Interrupt controller | QEMU PLIC | SiFive PLIC (ESWIN quirks) | Interrupt init may fail |
| MMU | QEMU simulation (Sv48 bug) | Real SiFive P550 | PTE A/D handling, TLB behavior differ |
| SBI | OpenSBI 1.5.1 (QEMU bundled) | Board OpenSBI (version unknown) | < 1.8 lacks EIC7700 PMP protection |
| Entry | OpenSBI → kernel | U-Boot `booti` → kernel | hartid in a0 from U-Boot, NOT mhartid CSR |
| Speculative exec | Not simulated | EIC7700 has known bus error issue | **Potential root cause of silent hang** |
| Physical memory | Continuous | NPU reserves 6 GB; holes | Available RAM differs |
| CPU count | Configurable | 4 × P550 + 1 × E21 | SMP init differences |

## 8. EIC7700 SPECULATIVE EXECUTION ISSUE (CRITICAL)

The SiFive P550 cores in EIC7700 speculatively access non-existent physical
addresses, triggering bus errors. This is a **hardware quirk**, not a kernel bug.

**Danger zones (single-die config)**:
- `0x1a00_0000 – 0x1a40_0000` — Die 0 L3 zero device
- `0x3a00_0000 – 0x3a40_0000` — Die 1 L3 zero device (absent in single-die)
- Memory port holes — remote die cache memory + gaps

**Fix**: OpenSBI ≥ 1.8 uses locked PMP entries to physically block these regions.

**Action**: Check OpenSBI version on the board:
```
=> sbi info
```
If version < 1.8, speculative bus errors may cause silent hangs that look
exactly like page table problems.

## 9. WHEN DIAGNOSING FAILURES

### If you see marker `Td:0` (Load Page Fault, VPN[3]=0)
→ Identity mapping access failed. Check boot_idpt entries.
→ Could be PA > 512 GiB or bug in gigapage PTE construction.

### If you see marker `Td:e` (Load Page Fault, VPN[3]=0xe=14→L4 index 14? No: e → 14 → L4[448])
Actually: VPN[3] is extracted as `stval >> 44`. For stval in the FRAME_METADATA range:
- `0xFFFF_E000_xxxx_xxxx` → bits [47:44] = `0xE` → marker shows `e`
→ FRAME_METADATA access failed. Check L4[448] → boot_meta_l3pt → boot_meta_l2 → boot_meta_l1_N chain.

### If you see marker `Td:f` (Load Page Fault, VPN[3]=15)
→ VPN[3]=15 = L4[511] = kernel high-half mapping failed.
→ Check L4[511] → boot_l3pt → L3[510] → boot_l2_kernel.

### If you see marker `Td:8` (Load Page Fault, VPN[3]=8)
→ VPN[3]=8 = L4[256] = linear mapping access failed.
→ Check L4[256] → boot_idpt.

### Complete silence after `AEFGD` on real hardware
Possible causes (in order of likelihood):
1. **EIC7700 speculative bus error** — CPU prefetcher hit a hole → bus error before trap handler can run
2. **Missing PTE_A on non-leaf** — HW walker requires A bit, QEMU doesn't
3. **DTB at unexpected address** — `0xf0000000` fallback failed
4. **UART base mismatch** — SBI putchar works but subsequent serial output doesn't

### Complete silence after `AEFGDJBWV` on real hardware
→ MMU enabled, VMA switch worked, but the jump to `riscv_boot` failed.
→ Check that `KERNEL_VMA_OFFSET` (0xffffffff00000000) matches the link address.
→ Check that boot_l2_kernel correctly covers the kernel's physical load address range.

### If QEMU 9.x/10.0.x shows Td in Sv48 but Sv39 works
→ This is the **known QEMU Sv48 TLB walker bug**. Not a kernel issue.
→ Options: (a) stick with Sv39 for QEMU, or (b) use QEMU 8.2.2 which handles Sv48 correctly.

### If no output at all on real hardware
1. Verify serial connection (`ls /dev/ttyUSB0`)
2. Verify U-Boot is at prompt (send newline, look for `=>`)
3. Verify `booti` image has correct magic (`python3 -c "import struct; d=open('img','rb').read(); print(hex(struct.unpack('<I',d[0x38:0x3c])[0]))"` — should be `0x05435352`)
4. Try loading the known-good Linux kernel to verify the boot chain works

## 10. CODE MODIFICATION GUIDELINES

When modifying boot.S or page table code:

1. **Always add diagnostic markers** for new code paths. Use SBI_PUTCHAR with a unique letter.
2. **Always SBI_SET_TIMER after marker output** to catch hangs.
3. **Never trust t0/t1 after SBI_SET_TIMER** — the macro clobbers them via rdtime.
4. **Always preserve a1 (DTB pointer) before any SBI ecall** — save to s1/s2.
5. **PTE construction for non-leaf entries**: PPN = (physical_addr >> 2) with PTE_V only (A/D bits optional but recommended).
6. **PTE construction for leaf entries**: Always set PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D.
7. **Page table mode must match `PagingConsts`**: If NR_LEVELS=4, ADDRESS_WIDTH=48 → use Sv48. If Sv39, change PagingConsts too.
8. **QEMU workarounds**: Use `#[cfg(target_arch = "riscv64")]` for temporary workarounds. Mark with `// QEMU workaround:` comment. Plan to remove before upstream submission.

## 11. COMMIT HISTORY MANAGEMENT

Current state: 49 commits with 15 session checkpoint commits and 10 Sv39↔Sv48 back-and-forth experiments.

**Before upstream submission**, the history must be cleaned. The recommended final commit series:

```
1.  porting: add Milk-V Megrez scheme, scripts, and docs
2.  riscv/boot: add SBI timer watchdog and fix phys/virt address handling
3.  riscv/boot: use cold reboot (type 1) and fix signed-constant bugs
4.  riscv/boot: fix SBI_SET_TIMER clobber and DTB validation
5.  riscv/boot: fix MMU setup and DTB parsing for QEMU 8.2.2
6.  riscv/boot: fix QEMU 8.2.2 ecall bug and add UART diagnostic markers
7.  riscv/boot: add identity-mapped Rust entry point
8.  riscv/boot: implement Sv39 page tables with identity DTB access
9.  riscv/boot: use 2 MiB leaves instead of 1 GiB gigapages
10. riscv/boot: call fill_early_info before memory region access
11. riscv/boot: use Sv39-only to reach kernel panic handler
    (squash: 3 Sv48 experiment commits into this)
12. riscv/serial: use legacy SBI putchar instead of sbi-rt
13. riscv: replace all sbi-rt calls with legacy SBI inline assembly
14. riscv: enable FPU by setting FS::Initial instead of FS::Off
15. riscv/panic: skip _Unwind_Backtrace on riscv64
16. riscv/boot: preserve DTB pointer across SBI ecall
17. riscv/boot: use non-leaf pointer for kernel mapping in Sv39
18. riscv: add FRAME_METADATA page table pre-allocation in boot.S
```

**Session summary commits to squash**: 15 commits with messages like "final session summary", "final proven state", "definitive QEMU boot test" — these are checkpoint markers, not meaningful units of change.

## 12. HARDWARE REFERENCE

### Milk-V Megrez / EIC7700

| Property | Value |
|----------|-------|
| CPU | SiFive P550, `rv64imafdch_zicsr_zifencei_zba_zbb_sscofpmf` |
| RAM | 16 GB LPDDR5 |
| UART | `snps,dw-apb-uart` at `0x50900000` |
| Network | `end1`, DHCP from Windows ICS, `192.168.100.2/24` |
| Serial | FTDI USB-UART, 115200 8N1, `/dev/ttyUSB0` in WSL |

### Board Credentials

| Account | Password | Notes |
|---------|----------|-------|
| `anjie` | `passwd` | Normal user, sudo |
| `root` | `milkv` | |

### EIC7700 Memory Map (key regions)

```
0x0000_0000 – 0x2000_0000      Die 0: P550 internal
0x4000_0000 – 0x6000_0000      Die 0: Low MMIO (System Port 0)
0x5090_0000                      UART0 base
0x8000_0000 – 0x10_8000_0000   Die 0: Cached DRAM (Memory Port)
0x40_0000_0000 – 0x60_0000_0000 Interleaved memory (cached)
0xc0_0000_0000 – 0xd0_0000_0000 Die 0: Non-cached DRAM
```

## 13. KEY DESIGN DECISIONS & RATIONALE

1. **Sv48 chosen over Sv39**: Future-proofing. P550 supports Sv48. QEMU Sv48 bugs are QEMU's problem, not ours. Sv48 gigapages are mandatory per spec (unlike Sv39).

2. **Legacy SBI over sbi-rt crate**: The `sbi-rt` crate's binary interface version check is incompatible with OpenSBI 1.5.1. Legacy SBI (EID 0x01 for putchar, 0x54494D45 for timer, 0x53525354 for reset) is simpler and universally supported.

3. **1 GiB gigapages for identity/linear, 2 MiB for kernel VMA**: Identity/linear maps need full PA coverage → gigapages are efficient. Kernel VMA needs precise 2 MiB leaves to work around QEMU 8.2.2 Sv39 gigapage non-support.

4. **`KERNEL_PT_READY` static**: During early boot, the boot page table may not have linear mapping ready. This flag gates `paddr_to_vaddr` to use identity mapping until `init_kernel_page_table` completes. This is a safety measure, not a permanent design.

5. **Separate identity + linear + kernel + meta mappings**: Four distinct L4 entries for four distinct purposes. This keeps the page table structure clean and debuggable.

## 14. USEFUL COMMANDS QUICK REFERENCE

```bash
# Check booti image header
xxd -l 64 aster-nix.board.booti

# Verify RISC-V Image magic
python3 -c "import struct; d=open('aster-nix.board.booti','rb').read(); print(hex(struct.unpack('<I',d[0x38:0x3c])[0]))"
# Should print: 0x05435352

# Check OpenSBI version (on board)
echo "sbi info" > /dev/ttyUSB0

# Dump kernel memory in U-Boot (on board)
# md 0x80200000 0x40

# Check DTB magic in U-Boot (on board)
# md 0xf0000000 0x4
# Should show: f0000000: edfe0dd0 (little-endian of 0xd00dfeed)

# List all RISC-V boot marker sequences in a log
grep -o 'AEFGD[^ ]*' /tmp/aster-qemu-run.log
```
