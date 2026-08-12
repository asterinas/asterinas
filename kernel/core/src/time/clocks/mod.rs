// SPDX-License-Identifier: MPL-2.0

pub(crate) use cpu_clock::*;
pub(crate) use system_wide::*;

mod cpu_clock;
mod system_wide;

pub(super) fn init() {
    system_wide::init();
}
