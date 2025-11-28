// SPDX-License-Identifier: MPL-2.0

pub mod cpu;
mod power;
pub mod signal;

pub fn init() {
    power::init();
}
