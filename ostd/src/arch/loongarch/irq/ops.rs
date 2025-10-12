// SPDX-License-Identifier: MPL-2.0

//! Interrupt operations.

// FIXME: Mark this as unsafe. See
// <https://github.com/asterinas/asterinas/issues/1120#issuecomment-2748696592>.
pub(crate) fn enable_local() {
    loongArch64::register::crmd::set_ie(true);
}

/// Enables local IRQs and halts the CPU to wait for interrupts.
///
/// This method guarantees that no interrupts can occur in the middle. In other words, IRQs must
/// either have been processed before this method is called, or they must wake the CPU up from the
/// halting state.
//
// FIXME: Mark this as unsafe. See
// <https://github.com/asterinas/asterinas/issues/1120#issuecomment-2748696592>.
pub(crate) fn enable_local_and_halt() {
    loongArch64::register::crmd::set_ie(true);
    // TODO: We should put the CPU into the idle state. However, doing so
    // without creating race conditions (see the doc comments above) in
    // LoongArch is challenging. Therefore, we now simply return here, as
    // spurious wakeups are acceptable for this method.
}

pub(crate) fn disable_local() {
    loongArch64::register::crmd::set_ie(false);
}

pub(crate) fn is_local_enabled() -> bool {
    loongArch64::register::crmd::read().ie()
}
