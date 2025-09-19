// SPDX-License-Identifier: MPL-2.0

//! The timer support.

use super::trap::TrapFrame;

// TODO: Add LoongArch timer support and call this method.
#[expect(dead_code)]
fn timer_callback(trapframe: &TrapFrame) {
    crate::timer::call_timer_callback_functions(trapframe);
}
