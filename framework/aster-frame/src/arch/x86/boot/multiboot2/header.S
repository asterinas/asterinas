/* SPDX-License-Identifier: MPL-2.0 */

// This is the GNU Multiboot 2 header.
// Reference: https://www.gnu.org/software/grub/manual/multiboot2/html_node/Index.html//Index
.section ".multiboot2_header", "a"
.code32

// Macros for cleaner code in the header fields.
MB2_MAGIC = 0xE85250D6
MB2_ARCHITECTURE = 0 // 32-bit (protected) mode of i386
MB2_HEADERLEN = header_end - header_start
MB2_CHECKSUM = -(MB2_MAGIC + MB2_ARCHITECTURE + MB2_HEADERLEN)
KERNEL_VMA = 0xffffffff80000000

header_start:
    .align 8

    .long MB2_MAGIC
    .long MB2_ARCHITECTURE
    .long MB2_HEADERLEN
    .long MB2_CHECKSUM

    // Tag: entry address
entry_address_tag_start:        
    .short 3
    .short 1                // Optional
    .long  entry_address_tag_end - entry_address_tag_start
.extern __multiboot_boot
    .long __multiboot_boot - KERNEL_VMA // entry_addr
entry_address_tag_end:

    // Tag: information request
    .align 8
info_request:
    .short 1
    .short 0            // Required
    .long  info_request_end - info_request
    .long  6            // Memory map request
    .long  15           // ACPI (new) request
info_request_end:

    // Tag: header end
    .align 8
    .short 0            // type: tags end
    .short 0            // flags
    .long 8             // size
header_end:
