// SPDX-License-Identifier: MPL-2.0

use ostd::info;

pub fn init() {
    for device in aster_input::all_devices() {
        info!("Found an input device, name: {}", device.name());
    }

    // FIXME: Currently, we have to do this manually to ensure the crates containing the input
    // devices are linked and their `#[init_component]` hooks can run to register the devices with
    // the input core. We should find a way to avoid this in the future.
    // Likewise, force the device-mapper component to be linked so its process-stage
    // `#[init_component]` hook can create mapped devices requested through the
    // `dm_mod.create=` kernel command line.
    use aster_device_mapper as _;
    #[expect(unused_imports)]
    use aster_i8042::*;
}
