/* SPDX-License-Identifier: MPL-2.0 */

// RISC-V BSP entrypoint for Milk-V Megrez / EIC7700.
//
// Assumptions:
//   - Entered in S-mode from U-Boot `booti` with a0=hartid, a1=DTB paddr.
//   - Do NOT read mhartid: it traps on EIC7700 in S-mode.
//   - SBI is still available (OpenSBI remains resident in M-mode).
//   - The kernel is loaded at physical 0x80200000 and linked at 0xffffffff80200000.
//
// Page table philosophy (shared with x86):
//   boot.S builds the MINIMAL page table needed to reach Rust.  All further
//   mappings — kernel VMA, FRAME_METADATA, linear extensions — are done
//   dynamically in Rust via `boot_pt::map_base_page()` and later the proper
//   kernel page table.  This keeps assembly small and auditable.
//
// Safety net:
//   A supervisor-timer interrupt is armed early. If anything hangs before the
//   kernel installs its own timer/trap handling, the timer fires into the tiny
//   trap handler below and the board is reset via SBI.

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

PTE_VRXWAD = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D

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

// Build a non-leaf PTE pointing to a sub-table.  The assembler sees VMA
// addresses for labels; we must strip the high 32 bits to get physical
// addresses (KERNEL_VMA_OFFSET = 0xffffffff00000000).
// Result in \dst_reg: (PA >> 2) | PTE_V.
.macro NONLEAF_PTE dst_reg, label
    lla    \dst_reg, \label
    slli   \dst_reg, \dst_reg, 32
    srli   \dst_reg, \dst_reg, 32
    srli   \dst_reg, \dst_reg, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    \dst_reg, \dst_reg, PTE_V
.endm

// Helper: get physical address of \label into \dst_reg.
.macro LABEL_PA dst_reg, label
    lla    \dst_reg, \label
    slli   \dst_reg, \dst_reg, 32
    srli   \dst_reg, \dst_reg, 32
.endm

.global _start
_start:
    // a0 = hart id, a1 = DTB paddr (from U-Boot / SBI).
    // Save a1 to s1/s2 BEFORE any SBI ecall: the SBI sbiret struct
    // (a0=error, a1=value) clobbers the firmware DTB pointer.
    mv     s0, a0
    mv     s1, a1
    mv     s2, a1

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
    // Fall back to s2 (original a1 snapshot, preserved across SBI ecalls).
    li     t0, 0xedfe0dd0          // FDT magic (little-endian of 0xd00dfeed)
    slli   t0, t0, 32
    srli   t0, t0, 32

    li     s1, 0xf0000000
    lwu    t1, 0(s1)
    beq    t0, t1, .Ldtb_ok

    li     t2, 0x80000000
    bltu   s2, t2, .Ldtb_hang
    lwu    t1, 0(s2)
    bne    t0, t1, .Ldtb_hang
    mv     s1, s2
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

    // --- Build the minimal Sv48 boot page table -------------------------------
    //
    // Three L4 entries to reach Rust:
    //   L4[0]   -> boot_idpt      — identity mapping (PA 0..512 GiB)
    //   L4[256] -> boot_idpt      — linear mapping (shared)
    //   L4[511] -> boot_l3pt      — kernel high-half (1 GiB identity leaves)
    //
    // L4[448] is set to boot_meta_l3pt — a single zero-filled L3 that lets
    // the MMU walk past L4 into the FRAME_METADATA region after paging is
    // enabled.  All deeper tables (L2→L1, leaf PTEs) are created dynamically
    // in Rust by boot_pt::map_base_page().
    //
    // boot_idpt uses 512 × 1 GiB gigapages.  Sv48 mandates gigapage support;
    // QEMU 8.2.2+ and SiFive P550 both handle them correctly.

    LABEL_PA t1, boot_l4pt

    // L4[0] -> boot_idpt (identity mapping)
    NONLEAF_PTE t0, boot_idpt
    sd       t0, 0(t1)

    // L4[256] -> boot_idpt (linear mapping, shared with identity)
    li       t2, 256 * PTE_SIZE
    add      t2, t1, t2
    sd       t0, 0(t2)

    // L4[511] -> boot_l3pt (kernel high-half)
    NONLEAF_PTE t0, boot_l3pt
    li       t2, 511 * PTE_SIZE
    add      t2, t1, t2
    sd       t0, 0(t2)

    // L4[448] -> boot_meta_l3pt (FRAME_METADATA region).
    // Must be set BEFORE the MMU is enabled so the page-table walker
    // doesn't fault on L4[448] when map_base_page first accesses it.
    NONLEAF_PTE t0, boot_meta_l3pt
    li       t2, 448 * PTE_SIZE
    add      t2, t1, t2
    sd       t0, 0(t2)

    // Replace boot_l3pt[510]'s 1 GiB leaf with a non-leaf pointer to
    // boot_l2_kernel (2 MiB leaves covering 0x80000000-0xbfffffff).
    // QEMU 9.x can reject 1 GiB leaf PTEs in Sv48 for kernel VMA.
    NONLEAF_PTE t0, boot_l2_kernel
    LABEL_PA t1, boot_l3pt
    li       t2, 510 * PTE_SIZE
    add      t2, t1, t2
    sd       t0, 0(t2)

    SBI_PUTCHAR 'D'
    SBI_SET_TIMER (60 * TIMER_FREQ)

    // Synchronise page-table writes before turning on the MMU.
    sfence.vma

    // Emit markers BEFORE enabling the MMU (safe via SBI ecall).
    SBI_PUTCHAR 'J'
    SBI_PUTCHAR 'B'

    // Disable the early safety-net timer.
    li     a0, -1
    li     a7, 0x54494D45             // SBI set_timer EID
    ecall

    // Enable Sv48 paging.
    // KERNEL_PT_READY=false → paddr_to_vaddr() uses identity mapping
    // during early boot, which boot_idpt fully supports.
    LABEL_PA t0, boot_l4pt
    li     t1, SATP_MODE_SV48
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
    // MMU is now on.  SBI ecall still works (OpenSBI in M-mode).
    SBI_SET_TIMER (90 * TIMER_FREQ)
    SBI_PUTCHAR 'W'

    // Switch to high-half virtual addresses.
    li     t1, KERNEL_VMA_OFFSET

    // Move early trap handler to virtual address.
    lla    t0, _early_trap
    or     t0, t0, t1
    csrw   stvec, t0

    // Switch stack pointer.
    lla    sp, boot_stack_top
    or     sp, sp, t1

    // Switch GP (CPU-local storage base).
    lla    gp, __cpu_local_start
    or     gp, gp, t1

    // Jump to Rust entry with VMA.
    lla    t0, riscv_boot
    or     t0, t0, t1
    mv     a0, s0                    // hart_id
    mv     a1, s1                    // DTB paddr

    SBI_PUTCHAR 'X'
    SBI_PUTCHAR 'V'

    jr     t0

    // Should never return.
1:  j      1b

// Tiny trap handler for the early-boot safety net.
//   - Supervisor timer interrupt → reset the board.
//   - Any other trap → print 'T' + scause low nibble + ':' + stval VPN[3] nibble,
//     then spin.  The hex nibbles are sufficient to diagnose page faults
//     (scause=0x0c/0x0d/0x0f), illegal instructions (0x02), and bus errors.
.balign 4
_early_trap:
    csrr   t0, scause
    li     t1, 0x8000000000000005
    beq    t0, t1, .Ltimer_reset

    // Print 'T' then scause low nibble
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

    // Print ':' separator
    li     a0, ':'
    li     a7, 0x01
    ecall

    // Print VPN[3] of stval (VA[47:44])
    csrr   t3, stval
    srli   t3, t3, 44
    andi   t4, t3, 0xf
    li     t5, 9
    ble    t4, t5, 3f
    addi   t4, t4, 'a' - 10
    j      4f
3:
    addi   t4, t4, '0'
4:
    mv     a0, t4
    li     a7, 0x01
    ecall
5:
    j      5b

.Ltimer_reset:
    SBI_RESET

// ---- Sv48 page tables ------------------------------------------------------

.balign 4096
boot_l4pt:
    // All entries initially zero.  L4[0], L4[256], L4[511] are filled at
    // runtime.  L4[448] stays zero — FRAME_METADATA mappings are created
    // dynamically by boot_pt::map_base_page() in Rust.
    .zero 512 * PTE_SIZE

// Identity and linear mappings share this table.
// 512 × 1 GiB leaf PTEs (gigapages) covering PA 0..512 GiB.
// Sv48 mandates gigapage support.
.balign 4096
boot_idpt:
    .set i, 0
    .rept 512
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr

// Single zero-filled L3 for FRAME_METADATA (L4[448] points here).
// boot_pt::map_base_page() dynamically creates L2/L1 tables on demand.
.balign 4096
boot_meta_l3pt:
    .zero 512 * PTE_SIZE

// High-half kernel mapping table (L4[511] selects this).
// 512 entries, each a 1 GiB identity leaf.
// Entry 510 is replaced at runtime with a non-leaf pointer to boot_l2_kernel.
.balign 4096
boot_l3pt:
    .set i, 0
    .rept 512
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr

// 2 MiB leaf pages for the kernel VMA region (L3[510] → this table,
// VA 0xffffffff80000000-0xffffffffbfffffff → phys 0x80000000-0xbfffffff).
.balign 4096
boot_l2_kernel:
    .set i, 0
    .rept 512
    .quad (((0x80000000 >> 12) + i * (0x200000 >> 12)) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr

// ---- Boot stack ------------------------------------------------------------
.section ".bss.stack", "aw", @nobits

.balign 4096
boot_stack_bottom:
    .skip 0x40000  // 256 KiB
boot_stack_top:
