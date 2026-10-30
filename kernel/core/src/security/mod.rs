// SPDX-License-Identifier: MPL-2.0

pub mod lsm;

#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
pub mod tdx;
#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
mod tsm_mr;

pub(super) fn init() {
    lsm::init();

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        tsm_mr::init();
    });
}
