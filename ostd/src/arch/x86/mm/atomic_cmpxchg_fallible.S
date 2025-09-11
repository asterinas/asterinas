/* SPDX-License-Identifier: MPL-2.0 */

.text
.code64

// Atomically compares and exchanges a 32-bit integer value. This function works with
// exception handling and can recover from a page fault.
//
// Returns the previous value or `!0u64` if failed to update.
.global __atomic_cmpxchg_fallible
__atomic_cmpxchg_fallible: # (ptr: *mut u32, old_val: u32, new_val: u32) -> u64
    mov eax, esi
.cmpxchg:
    lock; cmpxchg [rdi], edx
    ret
.cmpxchg_fault:
    mov rax, -1
    ret

.pushsection .ex_table, "a", @progbits
    .align 8
    .quad [.cmpxchg]
    .quad [.cmpxchg_fault]
.popsection
