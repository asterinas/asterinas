/* SPDX-License-Identifier: MPL-2.0 */

// Atomically loads a 32-bit integer value. This function works with exception handling
// and can recover from a page fault.
//
// Returns the loaded value or `!0u64` if failed to load.
.text
.global __atomic_load_fallible
.code64
__atomic_load_fallible: # (ptr: *const u32) -> u64;
.load:
    mov eax, [rdi]
    ret
.load_fault:
    mov rax, -1
    ret

.pushsection .ex_table, "a"
    .align 8
    .quad [.load]
    .quad [.load_fault]
.popsection
