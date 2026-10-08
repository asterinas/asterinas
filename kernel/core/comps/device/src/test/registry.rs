// SPDX-License-Identifier: MPL-2.0

//! Registry initialization tests.

use aster_systree::SysObj;
use ostd::prelude::ktest;

use super::{
    toy::{self, ToyDisk},
    utils,
};
use crate::{
    class::ClassDevice,
    common::{
        Error, SysStr,
        node::{Dir, SysTreeEdit},
        registry,
    },
};

#[ktest]
fn roots_exist() {
    crate::init_for_ktest();

    for path in [
        "/devices",
        "/devices/virtual",
        "/bus",
        "/class",
        "/dev/char",
        "/dev/block",
    ] {
        assert!(utils::lookup(path).is_some(), "{path} missing");
    }
}

#[ktest]
fn virtual_sysfs_nodes_share_class_directory() {
    let (class, _) = toy::register_toy_subsystems();
    let first = Dir::new(SysStr::from("sysfs-first"));
    crate::add_virtual_sysfs_node("toyblk", first.clone()).unwrap();
    assert_eq!(first.path(), "/devices/virtual/toyblk/sysfs-first");
    let directory = utils::lookup("/devices/virtual/toyblk").unwrap();

    let device = ClassDevice::builder(&class, "sysfs-disk", ToyDisk { sectors: 1 }).build();
    crate::add_device(&device).unwrap();
    assert_eq!(device.parent().unwrap().id(), directory.id());

    let second = Dir::new(SysStr::from("sysfs-second"));
    crate::add_virtual_sysfs_node("toyblk", second.clone()).unwrap();
    assert_eq!(second.parent().unwrap().id(), directory.id());
    assert!(matches!(
        crate::add_virtual_sysfs_node("toyblk", Dir::new(SysStr::from("sysfs-first"))),
        Err(Error::NameConflict)
    ));
    assert_eq!(
        utils::lookup("/devices/virtual/toyblk/sysfs-first")
            .unwrap()
            .id(),
        first.id()
    );

    crate::remove_device(&device).unwrap();
    assert!(utils::lookup("/devices/virtual/toyblk/sysfs-first").is_some());
    assert!(utils::lookup("/devices/virtual/toyblk/sysfs-second").is_some());

    let directory = directory.as_any().downcast_ref::<Dir>().unwrap();
    directory.detach_child("sysfs-first").unwrap();
    directory.detach_child("sysfs-second").unwrap();
    registry::get().drop_virtual_glue_dir_if_empty("toyblk");
    assert!(utils::lookup("/devices/virtual/toyblk").is_none());
}
