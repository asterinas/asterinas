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

    // The ELF note header: namesz, descsz, type.
    .long pvh_note_name_end - pvh_note_name_start
    .long pvh_note_desc_end - pvh_note_desc_start
    .long XEN_ELFNOTE_PHYS32_ENTRY

pvh_note_name_start:
    .asciz "Xen"
pvh_note_name_end:
    .align 4

.extern __pvh_boot
pvh_note_desc_start:
    // `__pvh_boot` lives in the `.bsp_boot` section, which is linked at its
    // physical load address, so the symbol value is already the physical
    // address that the VMM must jump to.
    .long __pvh_boot
pvh_note_desc_end:
    .align 4
