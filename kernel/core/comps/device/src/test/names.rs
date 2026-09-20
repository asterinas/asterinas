// SPDX-License-Identifier: MPL-2.0

//! Device name validation tests.

use ostd::prelude::ktest;

use super::toy::{self, ToyDev, ToyDisk};
use crate::{
    bus::BusDevice,
    class::ClassDevice,
    common::{BareDevice, DeviceInternals},
};

#[ktest]
fn valid_device_names_are_accepted() {
    // 1. Register the toy bus and class for constructing typed devices.
    let (class, bus) = toy::register_toy_subsystems();

    // 2. Check ordinary names, punctuation, and the `!` used in device paths.
    for name in ["disk0", "0000:00:01.0", "disk.part-1", "cciss!c0d0"] {
        let bare_dev = BareDevice::new_root(name);
        let bus_dev = BusDevice::builder(
            &bus,
            name,
            ToyDev {
                vendor: 1,
                model: 3,
            },
        )
        .build();
        let class_dev = ClassDevice::builder(&class, name, ToyDisk { sectors: 8 }).build();
        assert_eq!(bare_dev.base().name().as_ref(), name);
        assert_eq!(bus_dev.base().name().as_ref(), name);
        assert_eq!(class_dev.base().name().as_ref(), name);
    }
}

#[ktest]
#[should_panic]
fn empty_device_name_panics_at_construction() {
    BareDevice::new_root("");
}

#[ktest]
#[should_panic]
fn device_name_with_slash_panics_at_construction() {
    BareDevice::new_root("bad/name");
}

#[ktest]
#[should_panic]
fn device_name_with_nul_panics_at_construction() {
    BareDevice::new_root("bad\0name");
}

#[ktest]
#[should_panic]
fn dot_bus_device_name_panics_at_build() {
    // 1. Obtain the registered toy bus.
    let (_, bus) = toy::register_toy_subsystems();

    // 2. Building the device must reject the name before registration.
    BusDevice::builder(
        &bus,
        ".",
        ToyDev {
            vendor: 1,
            model: 3,
        },
    )
    .build();
}

#[ktest]
#[should_panic]
fn dot_dot_class_device_name_panics_at_build() {
    // 1. Obtain the registered toy class.
    let (class, _) = toy::register_toy_subsystems();

    // 2. Building the device must reject the name before registration.
    ClassDevice::builder(&class, "..", ToyDisk { sectors: 8 }).build();
}
