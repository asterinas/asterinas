/* SPDX-License-Identifier: MPL-2.0 */

// This is the GNU Multiboot header.
// Reference: https://www.gnu.org/software/grub/manual/multiboot/multiboot.html
.section ".multiboot_header", "a"

MB_MAGIC = 0x1BADB002
MB_FLAGS = 0
MB_CHECKSUM = -(MB_MAGIC + MB_FLAGS)

.code32
multiboot_header:
    .align 8
    .long MB_MAGIC
    .long MB_FLAGS
    .long MB_CHECKSUM
