.section ".setup", "ax"
.code64
.org 0x200
// start_of_setup64 should be at start_of_setup32 + 0x200
.global start_of_setup64
start_of_setup64:

.extern _trojan_entry_64
    lea rax, [rip + _trojan_entry_64]
    push rsi
    call rax
    
    // Unreachable here.
halt:
    hlt
    jmp halt
