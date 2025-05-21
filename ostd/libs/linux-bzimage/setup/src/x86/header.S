/* SPDX-License-Identifier: MPL-2.0 */

// The compatibility file for the Linux x86 Boot Protocol.
// See https://www.kernel.org/doc/html/v5.6/x86/boot.html for
// more information on the Linux x86 Boot Protocol.

// The bootloader may fill some fields at runtime, which can
// be read by the kernel (via `boot_params.hdr`).

.section ".header", "a"

CODE32_START = 0x100000

# Real-mode setup sectors. We don't use them. Their size is set to one page.
SETUP_SECTS = 7
SETUP_SECTS_SIZE = 0x200 * (SETUP_SECTS + 1)

.org 0x01f1
hdr_start:
setup_sects:            .byte SETUP_SECTS
root_flags:             .word 1
syssize:                .long 0
ram_size:               .word 0
vid_mode:               .word 0xfffd
root_dev:               .word 0
boot_flag:              .word 0xAA55
jump:                   .byte 0xeb
jump_addr:              .byte hdr_end - jump_addr
magic:                  .ascii "HdrS"
                        .word 0x020f
realmode_swtch:         .word 0, 0
start_sys_seg:          .word 0
                        .word 0
type_of_loader:         .byte 0
loadflags:              .byte (1 << 0)  # LOADED_HIGH
setup_move_size:        .word 0
code32_start:           .long CODE32_START
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

.if {CFG_TARGET_ARCH_X86_64}
# Note that we don't actually support the legacy 64-bit entry point
# (XLF_KERNEL_64). But we have to specify it, otherwise the boot loader
# will think this kernel does not have 64-bit support.
xloadflags:             .word 0b01011 # Bit 0: XLF_KERNEL_64
                                      # Bit 1: XLF_CAN_BE_LOADED_ABOVE_4G
                                      # Bit 3: XLF_EFI_HANDOVER_64
.else
xloadflags:             .word 0
.endif

cmdline_size:           .long 4096 - 1
hardware_subarch:       .long 0
hardware_subarch_data:  .quad 0
payload_offset:         .long 0  # Not used.
payload_length:         .long 0  # Not used.
setup_data:             .quad 0
pref_address:           .quad CODE32_START
init_size:              .long __executable_size

.if {CFG_TARGET_ARCH_X86_64}
handover_offset:        .long (entry_efi_handover32 - entry_legacy32)
.else
handover_offset:        .long 0
.endif

kernel_info_offset:     .long 0
hdr_end:
