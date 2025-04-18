// SPDX-License-Identifier: MPL-2.0

//! Configure the Interrupt Descriptor Table (IDT).

use alloc::boxed::Box;
use core::arch::global_asm;

use spin::Once;
use x86_64::{
    instructions::tables::lidt,
    structures::{idt::Entry, DescriptorTablePointer},
    PrivilegeLevel, VirtAddr,
};

global_asm!(include_str!("trap.S"));

const NUM_INTERRUPTS: usize = 256;

extern "C" {
    #[link_name = "trap_handler_table"]
    static VECTORS: [usize; NUM_INTERRUPTS];
}

static GLOBAL_IDT: Once<&'static [Entry<()>]> = Once::new();

/// Initializes and loads the IDT.
///
/// The caller should only call this method once in the boot context for each available processor.
/// This is not a safety requirement, however, because calling this method again will do nothing
/// more than load the same IDT.
pub(super) fn init() {
    let idt = *GLOBAL_IDT.call_once(|| {
        let idt = Box::leak(Box::new([const { Entry::missing() }; NUM_INTERRUPTS]));

        // SAFETY: The vector array is properly initialized, lives for `'static`, and will never be
        // mutated. So it's always fine to create an immutable borrow to it.
        let vectors = unsafe { &VECTORS };

        // Initialize the IDT entries.
        for (intr_no, &handler) in vectors.iter().enumerate() {
            let handler = VirtAddr::new(handler as u64);

            let entry = &mut idt[intr_no];
            // SAFETY: The handler defined in `trap.S` has a correct signature to handle the
            // corresponding exception or interrupt.
            let opt = unsafe { entry.set_handler_addr(handler) };

            // Enable `int3` and `into` in the userspace.
            if intr_no == 3 || intr_no == 4 {
                opt.set_privilege_level(PrivilegeLevel::Ring3);
            }
        }

        idt
    });

    let idtr = DescriptorTablePointer {
        limit: (core::mem::size_of_val(idt) - 1) as u16,
        base: VirtAddr::new(idt.as_ptr().addr() as u64),
    };
    // SAFETY: The IDT is valid to load because:
    //  - It lives for `'static`.
    //  - It contains correct entries at correct indexes: all handlers are defined in `trap.S` with
    //    correct handler signatures.
    unsafe { lidt(&idtr) };
}
