/* SPDX-License-Identifier: MPL-2.0 */

// RISC-V BSP entrypoint for Milk-V Megrez / EIC7700.
//
// Assumptions:
//   - Entered in S-mode from U-Boot `booti` with a0=hartid, a1=DTB paddr.
//   - Do NOT read mhartid: it traps on EIC7700 in S-mode.
//   - SBI is still available (OpenSBI remains resident in M-mode).
//   - The kernel is loaded at physical 0x80200000 and linked at 0xffffffff80200000.
//
// Safety net:
//   A supervisor-timer interrupt is armed early. If anything hangs before the
//   kernel installs its own timer/trap handling, the timer fires into the tiny
//   trap handler below and the board is reset via SBI.

SATP_MODE_SV39 = 8 << 60
SATP_MODE_SV48 = 9 << 60

PTE_V  = 0x01
PTE_R  = 0x02
PTE_W  = 0x04
PTE_X  = 0x08
PTE_A  = 0x40
PTE_D  = 0x80

PTE_PPN_SHIFT = 10
PTE_SIZE      = 8
PAGE_SHIFT    = 12

KERNEL_VMA_OFFSET = 0xffffffff00000000

// OpenSBI timebase on Milk-V Megrez is 1 MHz.
TIMER_FREQ = 1000000

.section ".text.entry", "ax", @progbits

// SBI legacy console putchar (EID = 0x01). Clobbers a0/a7.
.macro SBI_PUTCHAR ch
    li     a0, \ch
    li     a7, 0x01
    ecall
.endm

// Set the supervisor timer to (rdtime + delta). Clobbers a0/a6/a7/t0/t1.
// IMPORTANT: do NOT use t0/t1 after this macro without reloading them —
// both are clobbered by rdtime and the immediate load.
.macro SBI_SET_TIMER delta
    rdtime t0
    li     t1, \delta
    add    a0, t0, t1
    li     a6, 0
    li     a7, 0x54494D45          // SBI set_timer EID
    ecall
.endm

// Reset the board via SBI system reset (EID = 0x53525354).
.macro SBI_RESET
    li     a0, 1                   // reset_type = cold reboot
    li     a1, 0
    li     a6, 0
    li     a7, 0x53525354          // SBI system reset EID
    ecall
.endm

.global _start
_start:
    // a0 = hart id, a1 = DTB paddr (from U-Boot / SBI).
    mv     s0, a0
    mv     s1, a1

    // Install the tiny trap handler at its physical address.
    lla    t0, _early_trap
    slli   t0, t0, 32
    srli   t0, t0, 32
    csrw   stvec, t0
    SBI_SET_TIMER (60 * TIMER_FREQ)
    li     t0, 0x20
    csrrs  t1, sie, t0             // STIE
    csrsi  sstatus, 0x2            // SIE

    // --- DTB validation -------------------------------------------------------
    // Prefer 0xf0000000 (U-Boot's load address on real Megrez).
    // Fall back to firmware-provided pointer (a1 from OpenSBI / U-Boot).
    li     t0, 0xedfe0dd0          // FDT magic (little-endian of 0xd00dfeed)
    slli   t0, t0, 32
    srli   t0, t0, 32

    li     s1, 0xf0000000
    lwu    t1, 0(s1)
    beq    t0, t1, .Ldtb_ok

    li     t2, 0x80000000
    bltu   a1, t2, .Ldtb_hang
    lwu    t1, 0(a1)
    bne    t0, t1, .Ldtb_hang
    mv     s1, a1
.Ldtb_ok:
    mv     a0, s0
    mv     a1, s1
    SBI_PUTCHAR 'A'

    // Sanity-check that satp is accessible in S-mode.
    csrw   satp, zero
    SBI_PUTCHAR 'E'
    SBI_SET_TIMER (60 * TIMER_FREQ)
    csrr   t1, satp
    SBI_PUTCHAR 'F'
    SBI_SET_TIMER (60 * TIMER_FREQ)
    csrr   t1, satp
    SBI_PUTCHAR 'G'
    SBI_SET_TIMER (60 * TIMER_FREQ)

    // --- Build the Sv48 page table ------------------------------------------
    // Virtual address layout:
    //   IDENTITY:  0x00000000_00000000  -> phys,  L4[0]   -> boot_idpt
    //   LINEAR:    0xffff8000_00000000  -> phys,  L4[256] -> boot_idpt (shared)
    //   KERNEL:    0xffffffff_80000000  -> phys,  L4[511] -> boot_l3pt
    //
    // L4[0] and L4[256] share boot_idpt — VPN[2] of the linear base is 0,
    // so both L4 entries resolve to the same physical address via the same
    // 1 GiB leaf at boot_idpt[VPN[2]].  Avoid 512 GiB leaves at L4 because
    // QEMU (and some hardware) reject them.

    // L4[0] and L4[256] -> boot_idpt (non-leaf, V only).
    // Also fill boot_idpt[0..3] with non-leaf -> boot_l2_id_0..3.
    lla    t0, boot_idpt
    slli   t0, t0, 32
    srli   t0, t0, 32
    // Save boot_idpt phys addr for filling its entries.
    mv     s2, t0

    // Write boot_idpt entries 0..3.
    // Entry format: (phys_addr_of_L2 >> (PAGE_SHIFT - PTE_PPN_SHIFT)) | PTE_V

    lla    t0, boot_l2_id_0
    slli   t0, t0, 32; srli   t0, t0, 32
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V
    sd     t0,  0(s2)

    lla    t0, boot_l2_id_1
    slli   t0, t0, 32; srli   t0, t0, 32
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V
    sd     t0,  8(s2)

    lla    t0, boot_l2_id_2
    slli   t0, t0, 32; srli   t0, t0, 32
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V
    sd     t0, 16(s2)

    lla    t0, boot_l2_id_3
    slli   t0, t0, 32; srli   t0, t0, 32
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V
    sd     t0, 24(s2)

    // Write L4[0] and L4[256] pointing to boot_idpt.
    srli   t0, s2, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V

    lla    t1, boot_l4pt
    slli   t1, t1, 32; srli   t1, t1, 32
    sd     t0, 0(t1)               // L4[0]
    li     t2, 2048
    add    t2, t1, t2
    sd     t0, 0(t2)               // L4[256]

    // L4[511] -> boot_l3pt (non-leaf, V only — no A on non-leaf PTE).
    // Entry 510 is replaced below with a non-leaf pointer to boot_l2_kernel,
    // a table of 2 MiB leaf pages that QEMU handles correctly in Sv48.
    lla    t0, boot_l3pt
    slli   t0, t0, 32
    srli   t0, t0, 32
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V
    // offset = 511 * 8 = 4088.
    li     t2, 4088
    add    t2, t1, t2
    sd     t0, 0(t2)             // L4[511]

    // Replace L3[510]'s 1 GiB leaf with a non-leaf pointer to
    // boot_l2_kernel.  QEMU 9.x can reject 1 GiB leaf PTEs in Sv48.
    lla    t0, boot_l2_kernel
    slli   t0, t0, 32
    srli   t0, t0, 32
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V

    lla    t1, boot_l3pt
    slli   t1, t1, 32
    srli   t1, t1, 32
    // offset = 510 * 8 = 4080.
    li     t2, 4080
    add    t2, t1, t2
    sd     t0, 0(t2)             // L3[510]

    SBI_PUTCHAR 'D'
    SBI_SET_TIMER (60 * TIMER_FREQ)

    // Synchronise page-table writes before turning on the MMU.
    sfence.vma

    // Emit all markers BEFORE enabling the MMU.
    SBI_PUTCHAR 'J'
    SBI_PUTCHAR 'B'

    // Try Sv39 — QEMU 9.2.4 Sv48 TLB walker has a confirmed bug

    // KERNEL_PT_READY=false → paddr_to_vaddr uses identity during early boot.

    lla    t0, boot_l3pt_sv39
    slli   t0, t0, 32
    srli   t0, t0, 32
    li     t1, SATP_MODE_SV39
    srli   t0, t0, PAGE_SHIFT
    or     t0, t0, t1
    csrw   satp, t0
    sfence.vma

    csrr   t1, satp
    beq    t0, t1, .Lmmu_ok
    SBI_SET_TIMER (60 * TIMER_FREQ)
    j      .Lmmu_hang

.Ldtb_hang:
    SBI_PUTCHAR '@'
1:  j      1b

.Lmmu_hang:
    SBI_PUTCHAR '!'
1:  j      1b

.Lmmu_ok:
    // MMU is now on.  Kernel VMA uses 2 MiB leaf pages (boot_l2_kernel)
    // which QEMU 9.2.4 handles correctly including ecall returns.
    SBI_SET_TIMER (90 * TIMER_FREQ)

    // Emit a UART marker 'W' while still using identity mapping.
    // This confirms the identity PT is valid after MMU enable.
    // We use a different UART address path (through identity)
    // to distinguish from the VMA UART marker.
    li     t2, 0x10000000
    li     t3, 'W'
    sb     t3, 0(t2)

    // Now switch to VMA for the Rust jump.
    // Move early trap handler to virtual address.
    lla    t0, _early_trap
    li     t2, KERNEL_VMA_OFFSET
    or     t0, t0, t2
    csrw   stvec, t0

    // Switch SP/PC to the high-half virtual addresses.
    li     t1, KERNEL_VMA_OFFSET

    lla    sp, boot_stack_top
    or     sp, sp, t1

    lla    gp, __cpu_local_start
    or     gp, gp, t1

    lla    t0, riscv_boot
    or     t0, t0, t1
    mv     a0, s0
    mv     a1, s1

    // UART marker 'X' before VMA jump.
    // We're now executing from VMA, so this exercises the kernel
    // high-half page table via UART MMIO.
    li     t2, 0x10000000
    li     t3, 'X'
    sb     t3, 0(t2)

    // SBI marker 'V' via ecall - QEMU 9.2.4 handles this.
    SBI_PUTCHAR 'V'

    jr     t0

    // Should never return.
1:  j      1b

// Tiny trap handler:
//   - Supervisor timer interrupt -> reset the board (safety net).
//   - Anything else -> print 'T' + low nibble of scause and loop.
.balign 4
_early_trap:
    csrr   t0, scause
    li     t1, 0x8000000000000005
    beq    t0, t1, .Ltimer_reset

    li     a0, 'T'
    li     a7, 0x01
    ecall
    andi   t4, t0, 0xf
    li     t5, 9
    ble    t4, t5, 1f
    addi   t4, t4, 'a' - 10
    j      2f
1:
    addi   t4, t4, '0'
2:
    mv     a0, t4
    li     a7, 0x01
    ecall
3:
    j      3b

.Ltimer_reset:
    SBI_RESET

PTE_VRXWAD = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A
PTE_VRXW   = PTE_V | PTE_R | PTE_W | PTE_X        // no A/D — let HW set them

// ---- Sv48 page tables ------------------------------------------------------
.balign 4096
boot_l4pt:
    .zero 1 * PTE_SIZE           // L4[0]: set at runtime -> boot_idpt
    .zero 255 * PTE_SIZE         // L4[1..255]: unused
    .zero 1 * PTE_SIZE           // L4[256]: set at runtime -> boot_idpt
    .zero 254 * PTE_SIZE         // L4[257..510]: unused
    .zero 1 * PTE_SIZE           // L4[511]: set at runtime -> boot_l3pt

// Identity and linear mappings share this table.
// Entries VPN[0..3] are set at runtime to point to boot_l2_id_* tables
// (2 MiB leaf pages) covering phys 0..4 GiB.
// Entries VPN[4..511] are left zero (unmapped).
.balign 4096
boot_idpt:
    .zero 512 * PTE_SIZE

// Four L2 tables, each covering 1 GiB with 512 x 2 MiB leaf PTEs.
// Maps phys i*1GiB .. (i+1)*1GiB - 1 for identity/linear access.
.balign 4096
boot_l2_id_0:
    PTE_TMP_VRXWAD = PTE_VRXW
    .set PTE_VRXW, PTE_VRXW
    .set i, 0
    .rept 512
    .quad (((0x00000000 >> 12) + i * (0x200000 >> 12)) << PTE_PPN_SHIFT) | PTE_VRXW
    .set i, i + 1
    .endr

.balign 4096
boot_l2_id_1:
    .set i, 0
    .rept 512
    .quad (((0x40000000 >> 12) + i * (0x200000 >> 12)) << PTE_PPN_SHIFT) | PTE_VRXW
    .set i, i + 1
    .endr

.balign 4096
boot_l2_id_2:
    .set i, 0
    .rept 512
    .quad (((0x80000000 >> 12) + i * (0x200000 >> 12)) << PTE_PPN_SHIFT) | PTE_VRXW
    .set i, i + 1
    .endr

.balign 4096
boot_l2_id_3:
    .set i, 0
    .rept 512
    .quad (((0xC0000000 >> 12) + i * (0x200000 >> 12)) << PTE_PPN_SHIFT) | PTE_VRXW
    .set i, i + 1
    .endr

// High-half mapping for the kernel.
// L4[511] selects this table.
// All 512 entries (VPN[2]=0..511) are 1 GiB identity leaves.
// Entries 0..507 cover VA 0xfffffff000000000..0xfffffffee0000000 -> phys 0..508 GiB
// Entry 510 is set at runtime to point to boot_l2_kernel (2 MiB leaves)
// to avoid QEMU 9.x Sv48 1 GiB leaf PTE bugs in the kernel VMA region.
.balign 4096
boot_l3pt:
    .set i, 0
    .rept 512
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr

// 2 MiB leaf pages for the kernel VMA region (VPN[2]=510, covering
// VA 0xffffffff80000000-0xffffffffbfffffff -> phys 0x80000000-0xbfffffff).
// 512 entries, each covering 2 MiB.
.balign 4096
boot_l2_kernel:
    .set i, 0
    .rept 512
    .quad (((0x80000000 >> 12) + i * (0x200000 >> 12)) << PTE_PPN_SHIFT) | PTE_VRXW
    .set i, i + 1
    .endr

// ---- Sv39 fallback page table ----------------------------------------------
.balign 4096
boot_l3pt_sv39:
    // Identity mapping: VPN[0..127] -> phys 0..128 GiB (1 GiB leaves).
    .set i, 0
    .rept 128
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    // Unused gap: VPN[128..255].
    .zero 128 * PTE_SIZE
    // Linear mapping: VPN[256..383] -> phys 0..128 GiB (1 GiB leaves).
    // This makes paddr_to_vaddr() work under Sv39 — the linear-mapping
    // base (0xFFFF800000000000) has VPN[2]=256, so entries 256+phys>>30
    // resolve to the correct physical address.
    .set i, 0
    .rept 128
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    // Unused gap: VPN[384..507].
    .zero 124 * PTE_SIZE
    // Kernel high-half: VPN[508..511].
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff00000000 -> 0 GiB
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff40000000 -> 1 GiB
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff80000000 -> 2 GiB (kernel)
    .quad (0xc0000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffffc0000000 -> 3 GiB

// ---- Boot stack ------------------------------------------------------------
.section ".bss.stack", "aw", @nobits

.balign 4096
boot_stack_bottom:
    .skip 0x40000  // 256 KiB
boot_stack_top:
