// SPDX-License-Identifier: MPL-2.0

use device_id::DeviceId;

use crate::{
    device::{Device, DeviceType},
    prelude::*,
};

mod block;
pub(super) mod char;

pub(super) fn init_in_first_kthread() {
    block::init_in_first_kthread();
}

pub(super) fn init_in_first_process() -> Result<()> {
    block::init_in_first_process()?;

    Ok(())
}

pub fn lookup(device_type: DeviceType, device_id: DeviceId) -> Option<Arc<dyn Device>> {
    match device_type {
        DeviceType::Char => char::lookup(device_id),
        DeviceType::Block => block::lookup(device_id),
    }
}
