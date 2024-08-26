// SPDX-License-Identifier: MPL-2.0

//! This module maintains preemption-related information for the current task
//! on a CPU with a single 32-bit, CPU-local integer value.
//!
//! * Bits from 0 to 30 represents an unsigned counter called `guard_count`,
//!   which is the number of `DisabledPreemptGuard` instances held by the
//!   current CPU;
//! * Bit 31 is set to `!need_preempt`, where `need_preempt` is a boolean value
//!   that will be set by the scheduler when it decides that the current task
//!   _needs_ to be preempted.
//!
//! Thus, the current task on a CPU _should_ be preempted if and only if this
//! integer is equal to zero.
//!
//! The initial value of this integer is equal to `1 << 31`.
//!
//! This module provides a set of functions to access and manipulate
//! `guard_count` and `need_preempt`.

use crate::cpu_local_cell;

/// Returns whether the current task _should_ be preempted or not.
///
/// `should_preempt() == need_preempt() && get_guard_count() == 0`.
pub(in crate::task) fn should_preempt() -> bool {
    PREEMPT_INFO.load() == 0
}

#[allow(dead_code)]
pub(in crate::task) fn need_preempt() -> bool {
    PREEMPT_INFO.load() & NEED_PREEMPT_MASK == 0
}

pub(in crate::task) fn set_need_preempt() {
    PREEMPT_INFO.bitand_assign(!NEED_PREEMPT_MASK);
}

pub(in crate::task) fn clear_need_preempt() {
    PREEMPT_INFO.bitor_assign(NEED_PREEMPT_MASK);
}

pub(in crate::task) fn get_guard_count() -> u32 {
    PREEMPT_INFO.load() & GUARD_COUNT_MASK
}

pub(in crate::task) fn inc_guard_count() {
    PREEMPT_INFO.add_assign(1);
}

pub(in crate::task) fn dec_guard_count() {
    debug_assert!(get_guard_count() > 0);
    PREEMPT_INFO.sub_assign(1);
}

cpu_local_cell! {
    static PREEMPT_INFO: u32 = NEED_PREEMPT_MASK;
}

/// Resets the preempt info to the initial state.
///
/// # Safety
///
/// This function is only useful for the initialization of application
/// processors' CPU-local storage. Because that the BSP should access the CPU-
/// local storage (`PREEMPT_INFO`) (when doing heap allocation) before we can
/// initialize the CPU-local storage for APs, the value of the AP's
/// `PREEMPT_INFO` would be that of the BSP's. Therefore, we need to reset the
/// `PREEMPT_INFO` to the initial state on APs' initialization.
pub(crate) unsafe fn reset_preempt_info() {
    PREEMPT_INFO.store(NEED_PREEMPT_MASK);
}

const NEED_PREEMPT_MASK: u32 = 1 << 31;
const GUARD_COUNT_MASK: u32 = (1 << 31) - 1;
