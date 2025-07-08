/* SPDX-License-Identifier: MPL-2.0 */

.equ LOONGARCH_CSR_CRMD,            0x0     /* Current mode */
.equ LOONGARCH_CSR_PRMD,            0x1     /* Previous mode */
.equ LOONGARCH_CSR_EUEN,            0x2     /* Extended unit enable */
.equ LOONGARCH_CSR_PGDL,            0x19    /* Page table base address when VA[47] = 0 */
.equ LOONGARCH_CSR_PGDH,            0x1a    /* Page table base address when VA[47] = 1 */
.equ LOONGARCH_CSR_PGD,             0x1b    /* Page table base */
.equ LOONGARCH_CSR_PWCL,            0x1c    /* Page table walk control low */
.equ LOONGARCH_CSR_PWCH,            0x1d    /* Page table walk control high */
.equ LOONGARCH_CSR_STLBPS,          0x1e    /* STLB page size */
.equ LOONGARCH_CSR_CPUID,           0x20    /* CPUID */
.equ LOONGARCH_CSR_TLBRENTRY,       0x88    /* TLB refill exception entry */
.equ LOONGARCH_CSR_TLBRBADV,        0x89    /* TLB refill badvaddr */
.equ LOONGARCH_CSR_TLBRERA,         0x8a    /* TLB refill ERA */
.equ LOONGARCH_CSR_TLBRSAVE,        0x8b    /* KScratch for TLB refill exception */
.equ LOONGARCH_CSR_TLBRELO0,        0x8c    /* TLB refill entrylo0 */
.equ LOONGARCH_CSR_TLBRELO1,        0x8d    /* TLB refill entrylo1 */
.equ LOONGARCH_CSR_TLBREHI,         0x8e    /* TLB refill entryhi */
.equ LOONGARCH_CSR_DMW0,            0x180   /* Direct mapping window 0 */
.equ LOONGARCH_CSR_DMW1,            0x181   /* Direct mapping window 1 */
.equ LOONGARCH_CSR_DMW2,            0x182   /* Direct mapping window 2 */
.equ LOONGARCH_CSR_DMW3,            0x183   /* Direct mapping window 3 */
.equ SAVE_TLBREFILL_T1,             0x30    /* Save 0 */
.equ LOONGARCH_CSR_SAVE1,           0x31    /* Save 1 */
.equ LOONGARCH_CSR_SAVE2,           0x32    /* Save 2 */
.equ LOONGARCH_CSR_SAVE3,           0x33    /* Save 3 */
.equ LOONGARCH_CSR_SAVE4,           0x34    /* Save 4 */
.equ LOONGARCH_CSR_SAVE5,           0x35    /* Save 5 */
.equ LOONGARCH_CSR_SAVE6,           0x36    /* Save 6 */
.equ LOONGARCH_CSR_SAVE7,           0x37    /* Save 7 */

.section ".boot", "awx", @progbits
.globl _start
_start:
    # Set DMW0 (kernel)
    li.d    $t0, 0x9000000000000011     # CA, PLV0, 0x9000_xxxx_xxxx_xxxx
    csrwr   $t0, LOONGARCH_CSR_DMW0

    # Set DMW1 (kernel), temporary use
    li.d    $t0, 0x0000000000000011     # CA, PLV0, 0x0000_xxxx_xxxx_xxxx
    csrwr   $t0, LOONGARCH_CSR_DMW1

    # Set DMW2 (device)
    li.d    $t0, 0x8000000000000001     # UA, PLV0, 0x8000_xxxx_xxxx_xxxx
    csrwr   $t0, LOONGARCH_CSR_DMW2

    # Disable floating point unit etc.
    li.w    $t0, 0x00000000		        # FPE=0, SXE=0, ASXE=0, BTE=0
    csrwr   $t0, LOONGARCH_CSR_EUEN

    # Set the first level (root level) page table size
    li.w    $t0, 0x0000000c             # 4KB
    csrwr   $t0, LOONGARCH_CSR_STLBPS

    # Set the page size of the TLB refill
    # Now we only support the 4KB page size
    li.w    $t0, 0x0000000c             # 4KB
    csrwr   $t0, LOONGARCH_CSR_TLBREHI

    # Set the structure of page table
    # PTbase=12, PTwidth=9, Dir1_base=12 + 9, Dir1_width=9, Dir2_base=12 + 9 + 9, Dir2_width=9, PTEWidth=0 (64 bits)
    li.w    $t0, 12 | 9 << 5 | 21 << 10 | 9 << 15 | 30 << 20 | 9 << 25
    csrwr   $t0, LOONGARCH_CSR_PWCL
    # Dir3_base=12 + 9 + 9 + 9, Dir3_width=9
    li.w    $t0, 39 | 9 << 6
    csrwr   $t0, LOONGARCH_CSR_PWCH

    # Set the boot page table
    la   $t0, boot_l4pt
    csrwr   $t0, LOONGARCH_CSR_PGDL
    la   $t0, boot_l4pt
    csrwr   $t0, LOONGARCH_CSR_PGDH

    # Initialize TLB
    invtlb  0, $zero, $zero

    # Enable address translation and disable interrupts
    li.w    $t0, 0x00000010		        # PLV=0, IE=0, PG=1
    csrwr   $t0, LOONGARCH_CSR_CRMD
    li.w    $t0, 0x00000000		        # PPLV=0, PIE=0, PWE=0
    csrwr   $t0, LOONGARCH_CSR_PRMD

    # Set the entry of TLB refill exception
    la.global   $t0, _handle_tlb_refill
    # Convert the address to physical address
.extern KERNEL_VMA_OFFSET
    la.global   $t1, KERNEL_VMA_OFFSET
    sub.d   $t0, $t0, $t1
    csrwr   $t0, LOONGARCH_CSR_TLBRENTRY

    # Update SP/PC to use the virtual address
    la      $sp, boot_stack_top
    add.d   $sp, $sp, $t1
    la.global   $t0, _start_virt
    jr      $t0

.balign 4096
.globl boot_l4pt
boot_l4pt:
    .zero 8 * 512

.section ".boot.stack", "aw", @nobits
.globl boot_stack_bottom
boot_stack_bottom:
    .balign 4096
    .skip 0x40000  # 256 KiB
.globl boot_stack_top
boot_stack_top:

# From here, we're in the .text section: we no longer use physical address.
.text
.globl _start_virt
_start_virt:
    # Unset DMW1 (kernel)
    csrwr   $zero, LOONGARCH_CSR_DMW1

    # Initialize r21 to the CPU-local start address.
.extern __cpu_local_start
    la.global   $r21, __cpu_local_start

    # Jump to rust loongarch_boot
    la.global   $t0, loongarch_boot
    jr      $t0

.balign 4096
.text
.globl _handle_tlb_refill
_handle_tlb_refill:
    # Save $t0, $t1
    csrwr   $t0, LOONGARCH_CSR_TLBRSAVE
    csrwr   $t1, SAVE_TLBREFILL_T1

    # Read PGD
    csrrd   $t0, LOONGARCH_CSR_PGD

    # Walk page table
    lddir   $t0, $t0, 3
    andi    $t1, $t0, 0x1
    beqz    $t1, _invalid_pte
    li.d    $t1, 0x1
    andn    $t0, $t0, $t1

    lddir   $t0, $t0, 2
    andi    $t1, $t0, 0x1
    beqz    $t1, _invalid_pte
    li.d    $t1, 0x1
    andn    $t0, $t0, $t1

    lddir   $t0, $t0, 1
    andi    $t1, $t0, 0x1
    beqz    $t1, _invalid_pte
    li.d    $t1, 0x1
    andn    $t0, $t0, $t1

    # Load PTEs
    ldpte   $t0, 0
    ldpte   $t0, 1
    b       _fill_tlb

_invalid_pte:
    # For invalid PTE, set the PTE to 0, which is valid.
    csrwr   $zero, LOONGARCH_CSR_TLBRELO0
    csrwr   $zero, LOONGARCH_CSR_TLBRELO1

_fill_tlb:
    # Fill TLB
    tlbfill

    # Restore $t0, $t1
    csrrd   $t0, LOONGARCH_CSR_TLBRSAVE
    csrrd   $t1, SAVE_TLBREFILL_T1

    ertn
