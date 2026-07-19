// SPDX-License-Identifier: MPL-2.0

pub use cpu_clock::*;
pub use system_wide::*;

mod cpu_clock;
mod system_wide;

pub(super) fn init() {
    system_wide::init();
}
