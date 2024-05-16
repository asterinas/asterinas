// SPDX-License-Identifier: MPL-2.0

pub use system_wide::*;

mod system_wide;

pub(super) fn init() {
    system_wide::init();
}
