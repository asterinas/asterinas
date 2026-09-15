// SPDX-License-Identifier: MPL-2.0

pub mod lsm;

cfg_select! {
    all(target_arch = "x86_64", feature = "cvm_guest") => {
        mod tsm;
        mod tsm_mr;
    }
    _ => {}
}

pub(super) fn init() {
    lsm::init();

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        tsm::init();
        tsm_mr::init();
    });
}
