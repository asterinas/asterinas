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
//   trap handler below and the board is reset via SBI. This avoids requiring a
//   physical reset during early bring-up.

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
.macro SBI_SET_TIMER delta
    rdtime t0
    li     t1, \delta
    add    a0, t0, t1
    li     a6, 0
    li     a7, 0x54494D45          // SBI set_timer EID
    ecall
.endm

// Reset the board via SBI system reset (EID = 0x53525354). Clobbers a0/a1/a6/a7.
.macro SBI_RESET
    li     a0, 0                   // reset_type = cold reset
    li     a1, 0                   // reset_reason = 0
    li     a6, 0
    li     a7, 0x53525354          // SBI system reset EID
    ecall
.endm

.global _start
_start:
    // a0 = hart id, a1 = DTB paddr (from U-Boot / SBI).
    // Preserve them in callee-saved registers so SBI calls don't clobber them.
    mv     s0, a0
    mv     s1, a1

    // Install the tiny trap handler (physical address: we are still running
    // with the MMU off) and arm the watchdog timer. If anything hangs during
    // early boot, the timer will reset the board.
    lla    t0, _early_trap
    li     t1, KERNEL_VMA_OFFSET
    sub    t0, t0, t1
    csrw   stvec, t0
    SBI_SET_TIMER (60 * TIMER_FREQ)
    li     t0, 0x20
    csrrs  t1, sie, t0             // STIE
    csrsi  sstatus, 0x2            // SIE

    // Validate DTB pointer; fall back to the address we usually load it at.
    li     t0, 0xedfe0dd0          // FDT magic (little-endian)
    li     t2, 0x80000000
    bltu   a1, t2, .Luse_default_dtb
    lwu    t1, 0(a1)
    beq    t0, t1, .Ldtb_ok
.Luse_default_dtb:
    li     a1, 0xf0000000
    lwu    t1, 0(a1)
    bne    t0, t1, .Ldtb_hang
.Ldtb_ok:
    // SBI calls clobber a0/a1; restore the originals before proceeding.
    mv     a0, s0
    mv     a1, s1
    SBI_PUTCHAR 'A'
    SBI_SET_TIMER (60 * TIMER_FREQ)

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
    // L4[0]   -> 512 GiB identity leaf mapping physical 0
    // L4[256] -> 512 GiB linear leaf mapping physical 0
    // L4[511] -> boot_l3pt (high-half mapping for the kernel VMA)
    // All page-table symbols are linked high; convert to physical addresses.
    li     t4, KERNEL_VMA_OFFSET
    lla    t1, boot_l4pt
    sub    t1, t1, t4

    lla    t0, boot_l3pt
    sub    t0, t0, t4
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V | PTE_A  // non-leaf PTE: V+A (A may be required by HW walker)
    li     t2, 511 * PTE_SIZE
    add    t1, t1, t2
    sd     t0, 0(t1)               // L4[511]

    SBI_PUTCHAR 'D'
    SBI_SET_TIMER (60 * TIMER_FREQ)

    // Synchronise page-table writes before turning on the MMU.
    sfence.vma

    // Try Sv48.
    lla    t0, boot_l4pt
    li     t1, KERNEL_VMA_OFFSET
    sub    t0, t0, t1
    li     t1, SATP_MODE_SV48
    srli   t0, t0, PAGE_SHIFT
    or     t0, t0, t1
    csrw   satp, t0
    sfence.vma

    csrr   t1, satp
    SBI_PUTCHAR 'H'
    SBI_SET_TIMER (60 * TIMER_FREQ)
    beq    t0, t1, .Lmmu_ok

    // Try Sv39 fallback.
    lla    t0, boot_l3pt_sv39
    li     t1, KERNEL_VMA_OFFSET
    sub    t0, t0, t1
    li     t1, SATP_MODE_SV39
    srli   t0, t0, PAGE_SHIFT
    or     t0, t0, t1
    csrw   satp, t0
    sfence.vma

    csrr   t1, satp
    SBI_PUTCHAR 'h'
    SBI_SET_TIMER (60 * TIMER_FREQ)
    beq    t0, t1, .Lmmu_ok

.Ldtb_hang:
.Lmmu_hang:
    SBI_PUTCHAR '!'
1:  j      1b

.Lmmu_ok:
    SBI_PUTCHAR 'B'
    SBI_SET_TIMER (90 * TIMER_FREQ)   // cover early Rust code as well

    // The kernel is about to run at its linked virtual address; move the
    // early trap handler to the virtual address as well.
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
    // Restore the original hartid/DTB arguments for riscv_boot.
    mv     a0, s0
    mv     a1, s1
    jr     t0

// Tiny trap handler:
//   - Supervisor timer interrupt -> reset the board (safety net).
//   - Anything else            -> print 'T' + low nibble of scause and loop.
.balign 4
_early_trap:
    csrr   t0, scause
    li     t1, 0x8000000000000005    // supervisor timer interrupt
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

// ---- Sv48 page tables ------------------------------------------------------
.balign 4096
boot_l4pt:
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // L4[0]: identity 0-512 GiB
    .zero 255 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // L4[256]: linear 0-512 GiB
    .zero 254 * PTE_SIZE
    .quad 0                                          // L4[511]: -> boot_l3pt

boot_l3pt:
    // For KERNEL_VMA = 0xffffffff80200000 under Sv48:
    //   VPN3 = 511, VPN2 = 510 -> region base 0xffffffff80000000.
    //   Map that 1 GiB region to physical 0x80000000 (2 GiB) so that
    //   virtual 0xffffffff80200000 resolves to physical 0x80200000.
    .zero 508 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff00000000 -> 0 GiB
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff40000000 -> 1 GiB
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff80000000 -> 2 GiB (kernel)
    .quad (0xc0000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffffc0000000 -> 3 GiB

// ---- Sv39 fallback page table ----------------------------------------------
.balign 4096
boot_l3pt_sv39:
    // Identity mapping 0-128 GiB.
    .set i, 0
    .rept 128
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    .zero 128 * PTE_SIZE
    // Linear 0-128 GiB.
    .set i, 0
    .rept 128
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    // High-half mapping for 0xffffffff80200000 (Sv39 root index 510 -> phys 2 GiB).
    .zero 124 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRXWAD
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRXWAD
    .quad (0xc0000 << PTE_PPN_SHIFT) | PTE_VRXWAD

// ---- Boot stack ------------------------------------------------------------
.section ".bss.stack", "aw", @nobits

.balign 4096
boot_stack_bottom:
    .skip 0x40000  // 256 KiB
boot_stack_top:
