// SPDX-License-Identifier: MPL-2.0

use ostd::info;

pub(crate) fn init() {
    for device in aster_input::all_devices() {
        info!("Found an input device, name: {}", device.name());
    }
}
