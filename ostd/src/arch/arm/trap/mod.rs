// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

#[expect(clippy::module_inception)]
mod trap;

pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

/// Initializes interrupt handling on ARM.
///
/// # Safety
///
/// On the current CPU, this function must be called
/// - only once and
/// - before any trap can occur.
pub(crate) unsafe fn init_on_cpu() {
    // SAFETY: The caller ensures the safety conditions.
    unsafe {
        trap::init_on_cpu();
    }
}
