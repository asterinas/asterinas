// SPDX-License-Identifier: MPL-2.0

//! ASID (Address Space ID) support for x86.

use core::{
    arch::asm,
    sync::atomic::{AtomicBool, Ordering},
};

/// Global flag indicating if PCID is enabled
pub static PCID_ENABLED: AtomicBool = AtomicBool::new(false);

/// The maximum ASID value supported by hardware.
///
/// The PCID (Process-Context Identifier) on x86-64 architectures is 12 bits.
/// This means the maximum ASID value is 2^12-1 = 4095.
/// We reserve 0 for the kernel.
pub const ASID_CAP: u16 = 4095;

/// invpcid instruction:
/// INVPCID_TYPE := value of register operand; // must be in the range of 0–3
/// INVPCID_DESC := value of memory operand;
/// CASE INVPCID_TYPE OF
///     0:
///             // individual-address invalidation
///         PCID := INVPCID_DESC[11:0];
///         L_ADDR := INVPCID_DESC[127:64];
///         Invalidate mappings for L_ADDR associated with PCID except global translations;
///         BREAK;
///     1:
///             // single PCID invalidation
///         PCID := INVPCID_DESC[11:0];
///         Invalidate all mappings associated with PCID except global translations;
///         BREAK;
///     2:
///             // all PCID invalidation including global translations
///         Invalidate all mappings for all PCIDs, including global translations;
///         BREAK;
///     3:
///             // all PCID invalidation retaining global translations
///         Invalidate all mappings for all PCIDs except global translations;
///         BREAK;
/// ESAC;
enum InvpcidType {
    /// Invalidate mappings for L_ADDR associated with PCID except global translations;
    IndividualAddressInvalidation,
    /// Invalidate all mappings associated with PCID except global translations;
    SinglePcidInvalidation,
    /// Invalidate all mappings for all PCIDs, including global translations;
    AllPcidInvalidation,
    /// Invalidate all mappings for all PCIDs except global translations;
    AllPcidInvalidationRetainingGlobal,
}

/// Internal function to execute INVPCID with given parameters
unsafe fn invpcid_internal(type_: u64, asid: u64, addr: u64) {
    if !PCID_ENABLED.load(Ordering::Relaxed) {
        // Fallback for systems without PCID support
        match type_ as usize {
            // IndividualAddressInvalidation
            0 => {
                asm!(
                    "invlpg [{}]",
                    in(reg) addr,
                    options(nostack),
                );
            }
            // SinglePcidInvalidation - flush all non-global
            1 => super::tlb_flush_all_excluding_global(),
            // AllPcidInvalidation - flush all including global
            2 => super::tlb_flush_all_including_global(),
            // AllPcidInvalidationRetainingGlobal - flush all non-global
            3 => super::tlb_flush_all_excluding_global(),
            _ => panic!("Invalid INVPCID type"),
        }
        return;
    }

    // Use INVPCID if supported
    let descriptor = [addr, asid];
    unsafe {
        asm!(
            "invpcid {0}, [{1}]",
            in(reg) type_,
            in(reg) &descriptor,
            options(nostack),
        );
    }
}

/// Invalidate a TLB entry for a specific ASID and virtual address.
///
/// # Safety
///
/// This is a privileged instruction that must be called in kernel mode.
pub unsafe fn invpcid_single_address(asid: u16, addr: usize) {
    invpcid_internal(
        InvpcidType::IndividualAddressInvalidation as u64,
        asid as u64,
        addr as u64,
    );
}

/// Invalidate all TLB entries for a specific ASID.
///
/// # Safety
///
/// This is a privileged instruction that must be called in kernel mode.
pub unsafe fn invpcid_single_context(asid: u16) {
    invpcid_internal(InvpcidType::SinglePcidInvalidation as u64, asid as u64, 0);
}

/// Invalidate all TLB entries for all contexts.
///
/// # Safety
///
/// This is a privileged instruction that must be called in kernel mode.
pub unsafe fn invpcid_all_excluding_global() {
    invpcid_internal(InvpcidType::AllPcidInvalidationRetainingGlobal as u64, 0, 0);
}

/// Invalidate all TLB entries for all contexts, including global translations.
///
/// # Safety
///
/// This is a privileged instruction that must be called in kernel mode.
pub unsafe fn invpcid_all_including_global() {
    invpcid_internal(InvpcidType::AllPcidInvalidation as u64, 0, 0);
}
