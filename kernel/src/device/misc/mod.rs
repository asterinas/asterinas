// SPDX-License-Identifier: MPL-2.0

//! Misc devices.
//!
//! Character device with major number 10.

use device_id::MajorId;
use spin::Once;

use super::registry::char::{acquire_major, MajorIdOwner};

#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
pub mod tdxguest;

static MISC_MAJOR: Once<MajorIdOwner> = Once::new();

pub(super) fn init_in_first_kthread() {
    MISC_MAJOR.call_once(|| acquire_major(MajorId::new(10)).unwrap());

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        super::registry::char::register(tdxguest::TdxGuest::new()).unwrap();
    });
}
