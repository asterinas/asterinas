// SPDX-License-Identifier: MPL-2.0

//! Misc devices.
//!
//! Character device with major number 10.
//!
//! See <https://www.kernel.org/doc/Documentation/admin-guide/devices.txt>.

use aster_device::{register_device_ids, DeviceIdAllocator, DeviceType};
use spin::Once;

#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
pub mod tdxguest;

const MISC_MAJOR: u32 = 10;

static MISC_ID_ALLOCATOR: Once<DeviceIdAllocator> = Once::new();

pub(super) fn init_in_first_process() {
    let ida = register_device_ids(DeviceType::Char, MISC_MAJOR, 0..256).unwrap();
    MISC_ID_ALLOCATOR.call_once(|| ida);

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        use crate::fs::device::add_device;

        add_device(tdxguest::TdxGuest::new());
    });
}
