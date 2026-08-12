// SPDX-License-Identifier: MPL-2.0

pub(crate) mod cpu;
mod power;
pub(crate) mod ptrace;
pub(crate) mod signal;

pub(crate) fn init() {
    power::init();
}
