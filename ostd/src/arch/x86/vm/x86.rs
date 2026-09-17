// SPDX-License-Identifier: MPL-2.0

use x86::{dtables::DescriptorTablePointer, segmentation::SegmentSelector};

pub(super) fn write_cr2_raw(value: u64) {
    // SAFETY: CR2 records a fault address; writing it changes no mappings or
    // access rights. Guest execution swaps it while local IRQs are disabled.
    unsafe {
        core::arch::asm!(
            "mov cr2, {}",
            in(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }
}

/// Reads the base of the current CPU's task-state segment.
///
/// # Safety
///
/// `tr` and `gdt` must describe the current host's live TSS and GDT, which must
/// remain unchanged and accessible for the duration of this call.
pub(super) unsafe fn get_tr_base(tr: SegmentSelector, gdt: &DescriptorTablePointer<u64>) -> u64 {
    let offset = usize::from(tr.bits() & !7);
    // SAFETY: The caller guarantees that this is a live 16-byte TSS descriptor.
    let (low, high) = unsafe {
        let descriptor = gdt.base.cast::<u8>().add(offset).cast::<u64>();
        (
            descriptor.read_unaligned(),
            descriptor.add(1).read_unaligned(),
        )
    };
    ((low >> 16) & 0xff_ffff) | ((low >> 32) & 0xff00_0000) | (high << 32)
}
