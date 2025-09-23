// SPDX-License-Identifier: MPL-2.0

#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
mod tsm;

pub(super) fn init() {
    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    tsm::init();
}
