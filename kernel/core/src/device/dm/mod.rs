// SPDX-License-Identifier: MPL-2.0

//! Device Mapper device entry points.

use aster_dm::DmManager;
use spin::Once;

mod control;

static DM_MANAGER: Once<DmManager> = Once::new();

pub(super) fn init_in_first_kthread() {
    // Reserve the block major before exposing the control device to userspace.
    DM_MANAGER.call_once(|| DmManager::new().unwrap());
    control::init_in_first_kthread();
}
