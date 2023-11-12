// 32-bit setup code starts here, and will be loaded at CODE32_START.
.section ".setup", "ax"
.code32
.global start_of_setup32
start_of_setup32:
        mov eax, offset stack_bottom
        mov esp, eax
        mov eax, offset halt
        push eax                        # the return address
        mov ebp, esp
        add ebp, -4
        push ebp
        mov ebp, esp

        .extern _rust_setup_entry
        push esi                        # the boot_params pointer
        call _rust_setup_entry

        // Unreachable here.
halt:
        hlt
        jmp halt

.code64
.global start_of_setup64
.org 0x200
start_of_setup64:
        // Unreachable here.
halt64:
        hlt
        jmp halt64

// A small stack for the legacy setup code.
.section ".stack", "aw"
SETUP_STACK_SIZE = 0x1000
        .align 16
        stack_top:
        .skip SETUP_STACK_SIZE
        stack_bottom:
