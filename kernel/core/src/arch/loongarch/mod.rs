// SPDX-License-Identifier: MPL-2.0

pub(crate) mod cpu;
pub(crate) mod power;
pub(crate) mod signal;

pub(crate) fn init() {
    power::init();
}
