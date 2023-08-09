// The compatibility file for the Linux x86 Boot Protocol.
// See https://www.kernel.org/doc/html/v5.6/x86/boot.html for
// more information on the Linux x86 Boot Protocol.

.intel_syntax noprefix

// The section name is used by the build script to strip and make
// the binary file.
.section ".boot_compatibility_bin", "awx"

// The Linux x86 Boot Protocol header.
// 
// Some of the fields filled with a 0xab* values should be filled
// by the runner, which is the only tool after building and can
// access the info of the payload.
// Jinux will use only a few of these fields, and some of them
// are filled by the loader and will be read by Jinux.

.code16

sentinel:               .byte 0xff, 0xff
.org 0x01f1
hdr:
setup_sects:            .byte 0
root_flags:             .word 1
syssize:                .long 0
ram_size:               .word 0
vid_mode:               .word 0xfffd
root_dev:               .word 0
boot_flag:              .word 0xAA55
jump:                   .byte 0xeb
                        .byte start_of_setup-jump
magic:                  .ascii "HdrS"
                        .word 0x020f
realmode_swtch:         .word 0, 0
start_sys_seg:          .word 0
                        .word 0
type_of_loader:         .byte 0
loadflags:              .byte (1 << 0)
setup_move_size:        .word 0x8000
code32_start:           .long 0x100000
ramdisk_image:          .long 0
ramdisk_size:           .long 0
bootsect_kludge:        .long 0
heap_end_ptr:           .word 65535
ext_loader_ver:         .byte 0
ext_loader_type:        .byte 0
cmd_line_ptr:           .long 0
initrd_addr_max:        .long 0x7fffffff
kernel_alignment:       .long 0x1000000
relocatable_kernel:     .byte 0
min_alignment:          .byte 0x10
xloadflags:             .word 0 # none of the flags supported
cmdline_size:           .long 4096-1
hardware_subarch:       .long 0
hardware_subarch_data:  .quad 0
payload_offset:         .long 0xabababab # at 0x248/4, to be filled by the runner
payload_length:         .long 0xabababab # at 0x24c/4, to be filled by the runner
setup_data:             .quad 0
pref_address:           .quad 0
init_size:              .long 0
handover_offset:        .long 0
kernel_info_offset:     .long 0

// End of header.

// Temporary real mode GDTR/GDT entries.
.align 16
real_gdtr:
        .word gdt_end - gdt - 1
        .quad gdt

.align 16
gdt:
        .quad 0x0000000000000000 # 0:  null descriptor
        .quad 0x00cf8a000000ffff # 8:  32-bit system segment (4k sys ex rw)
        .quad 0x00cf9a000000ffff # 16:  32-bit code/data segment (4k sys ex rw)
gdt_end:

// 16-bit setup code starts here.
.code16
start_of_setup:
// Enter 32-bit protected mode without paging.
        // Disable interrupts.
        cli
        
        // Enable a20 gate.
        in al, 0x92
        or al, 2
        out 0x92, al

        // Load GDT.
        lgdt [real_gdtr]
        mov eax, cr0
        or eax, 1
        mov cr0, eax

        // Go to protected mode.
        jmp start_of_setup32

// 32-bit setup code starts here.
.code32
start_of_setup32:
        // print to screen a debug message using out port 0x3f8.
        mov dx, 0x3f8
        mov al, 'H'
        out dx, al
        mov al, 'e'
        out dx, al
        mov al, 'l'
        out dx, al
        mov al, 'l'
        out dx, al
        mov al, 'o'
        out dx, al
