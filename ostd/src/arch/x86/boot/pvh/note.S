/* SPDX-License-Identifier: MPL-2.0 */

// The PVH ELF note.
//
// VMMs that implement the PVH boot protocol (e.g., Cloud Hypervisor,
// Firecracker, QEMU) discover the kernel's 32-bit entry point by scanning the
// PT_NOTE segments of the ELF image for a note named "Xen" with the type
// `XEN_ELFNOTE_PHYS32_ENTRY`. The descriptor holds the physical address to
// jump to.
//
// Reference: https://xenbits.xen.org/docs/unstable/misc/pvh.html

XEN_ELFNOTE_PHYS32_ENTRY = 18

.section ".note.Xen", "a", @note
.align 4

    .long name_end - name_start     // namesz
    .long desc_end - desc_start     // descsz
    .long XEN_ELFNOTE_PHYS32_ENTRY  // type

name_start:
    .asciz "Xen"
name_end:
    .align 4

.extern __pvh_boot
desc_start:
    // `__pvh_boot` lives in the `.bsp_boot` section, which is linked at its
    // physical load address, so the symbol value is already the physical
    // address that the VMM must jump to.
    .long __pvh_boot
desc_end:
    .align 4
