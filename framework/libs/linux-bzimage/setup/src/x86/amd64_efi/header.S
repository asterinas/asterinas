/* SPDX-License-Identifier: MPL-2.0 */

// The compatibility file for the Linux x86 Boot Protocol.
// See https://www.kernel.org/doc/html/v5.6/x86/boot.html for
// more information on the Linux x86 Boot Protocol.

// Some of the fields filled with a 0xab* values should be filled
// by the torjan builder.
// Asterinas will use only a few of these fields, and some of them
// are filled by the loader and will be read by Asterinas.

.section ".header", "a"
CODE32_START = 0x100000
SETUP_SECTS = 7         # so that the legacy setup could occupy a page
SETUP_SECTS_SIZE = 0x200 * (SETUP_SECTS + 1)
.code16
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
jump_addr:              .byte hdr_end-jump_addr
magic:                  .ascii "HdrS"
                        .word 0x020f
realmode_swtch:         .word 0, 0
start_sys_seg:          .word 0
                        .word 0
type_of_loader:         .byte 0
loadflags:              .byte (1 << 0)
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
xloadflags:             .word 0b01111 # all handover protocols except kexec
cmdline_size:           .long 4096-1
hardware_subarch:       .long 0
hardware_subarch_data:  .quad 0
payload_offset:         .long 0xabababab # at 0x248/4, to be filled by the builder
payload_length:         .long 0xabababab # at 0x24c/4, to be filled by the builder
setup_data:             .quad 0
pref_address:           .quad CODE32_START - SETUP_SECTS_SIZE
init_size:              .long 0xabababab # at 0x260/4, to be filled by the builder
# The handover_offset should be efi_handover_setup_entry - CODE32_START - 0x200
# But we use ABI workaround to avoid the relocation of efi_handover_setup_entry
handover_offset:        .long 0x10
kernel_info_offset:     .long 0
hdr_end:
