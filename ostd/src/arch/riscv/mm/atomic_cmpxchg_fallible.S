/* SPDX-License-Identifier: MPL-2.0 */

.text

.option push
# We explicitly add this directive due to a known bug in how rustc treats target
# features in `global_asm`. Check https://github.com/rust-lang/rust/issues/111637#issuecomment-1870096878
# for details.
.option arch, rv64imac

# Atomically compares and exchanges a 32-bit integer value. This function works
# with exception handling and can recover from a page fault.
#
# Returns the previous value or `!0u64` if failed to update.
.global __atomic_cmpxchg_fallible
.type __atomic_cmpxchg_fallible, @function
__atomic_cmpxchg_fallible: # (ptr: *mut u32, old_val: u32, new_val: u32) -> u64
    li t2, 0b1 << 18
    csrs sstatus, t2
cmpxchg_load:
    lr.w t0, (a0)
    bne t0, a1, cmpxchg_done
cmpxchg_store:
    sc.w t1, a2, (a0)
    bnez t1, cmpxchg_load
cmpxchg_done:
    mv a0, t0
    csrc sstatus, t2
    ret
cmpxchg_fault:
    li a0, -1
    csrc sstatus, t2
    ret
.size __atomic_cmpxchg_fallible, .-__atomic_cmpxchg_fallible

.option pop

.pushsection .ex_table, "a"
    .balign 8
    .quad cmpxchg_load
    .quad cmpxchg_fault
    .quad cmpxchg_store
    .quad cmpxchg_fault
.popsection
