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
    lla    t0, boot_idpt
    slli   t0, t0, 32
    srli   t0, t0, 32
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V

    lla    t1, boot_l4pt
    slli   t1, t1, 32
    srli   t1, t1, 32
    // L4[0]
    sd     t0, 0(t1)
    // L4[256]: offset = 256 * 8 = 2048.
    // sd's 12-bit signed immediate only reaches [-2048, 2047]; use
    // base+base+add to reach the entry.
    li     t2, 2048
    add    t2, t1, t2
    sd     t0, 0(t2)

    // L4[511] -> boot_l3pt (non-leaf, V+A).
    lla    t0, boot_l3pt
    slli   t0, t0, 32
    srli   t0, t0, 32
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V | PTE_A
    // offset = 511 * 8 = 4088.
    li     t2, 4088
    add    t2, t1, t2
    sd     t0, 0(t2)             // L4[511]

    SBI_PUTCHAR 'D'
    SBI_SET_TIMER (60 * TIMER_FREQ)

    // Synchronise page-table writes before turning on the MMU.
    sfence.vma

    // Emit all markers BEFORE enabling the MMU.  QEMU 8.2.2 has a bug
    // where Sv48 page walks after an ecall return can fail.  Printing
    // markers here (with MMU off) avoids triggering that bug.
    SBI_PUTCHAR 'J'
    SBI_PUTCHAR 'B'

    // Try Sv48 first — the linear-mapping base (0xFFFF800000000000)
    // is only canonical under Sv48 and above.
    lla    t0, boot_l4pt
    slli   t0, t0, 32
    srli   t0, t0, 32
    li     t1, SATP_MODE_SV48
    srli   t0, t0, PAGE_SHIFT
    or     t0, t0, t1
    csrw   satp, t0
    sfence.vma

    csrr   t1, satp
    beq    t0, t1, .Lmmu_ok
    SBI_SET_TIMER (60 * TIMER_FREQ)

    // Try Sv39 fallback.
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
    // MMU is now on.  On QEMU 8.2.2, Sv48 ecall returns can trigger
    // spurious page faults.  The kernel page table (set up later by
    // call_ostd_main) uses 4 KiB leaves and works correctly with SBI.
    // Stay quiet until the full kernel PT is active.
    SBI_SET_TIMER (90 * TIMER_FREQ)

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

    // Emit a final marker via UART (not SBI), so we can confirm the
    // virtual-mode jump landed.  QEMU virt has NS16550 at 0x10000000;
    // the real Megrez has dw-apb-uart at 0x50900000.  We write the
    // UART_THR regardless — on QEMU this gives us visibility; on
    // real hardware it may not produce output but is harmless.
    li     t2, 0x10000000
    li     t3, 'X'
    sb     t3, 0(t2)

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

PTE_VRXWAD = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D
PTE_VRXW  = PTE_V | PTE_R | PTE_W | PTE_X

// ---- Sv48 page tables ------------------------------------------------------
.balign 4096
boot_l4pt:
    .zero 1 * PTE_SIZE           // L4[0]: set at runtime -> boot_idpt
    .zero 255 * PTE_SIZE         // L4[1..255]: unused
    .zero 1 * PTE_SIZE           // L4[256]: set at runtime -> boot_idpt
    .zero 254 * PTE_SIZE         // L4[257..510]: unused
    .zero 1 * PTE_SIZE           // L4[511]: set at runtime -> boot_l3pt

// Identity and linear mappings share this table.
// Entries VPN[0..127] -> phys 0..128 GiB via 1 GiB leaf pages.
// Entries VPN[128..511] are left zero (unmapped).
.balign 4096
boot_idpt:
    .set i, 0
    .rept 128
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    .zero (512 - 128) * PTE_SIZE

// High-half mapping for the kernel.
// L4[511] selects this table; only the last four entries are populated to
// cover VA range 0xffffffff00000000..0xffffffffffffffff.
.balign 4096
boot_l3pt:
    .zero 508 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff00000000 -> 0 GiB
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff40000000 -> 1 GiB
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff80000000 -> 2 GiB (kernel)
    .quad (0xc0000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffffc0000000 -> 3 GiB

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
