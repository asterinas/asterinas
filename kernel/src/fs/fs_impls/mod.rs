// SPDX-License-Identifier: MPL-2.0

//! Concrete file system implementations.
//!
//! This module contains all the specific file system implementations supported by the kernel.

pub mod cgroupfs;
pub mod configfs;
pub mod devpts;
pub mod exfat;
pub mod ext2;
pub mod overlayfs;
pub mod procfs;
pub mod pseudofs;
pub mod ramfs;
pub mod sysfs;
pub mod tmpfs;

pub(super) fn init() {
    sysfs::init();
    procfs::init();
    cgroupfs::init();
    configfs::init();
    ramfs::init();
    tmpfs::init();
    devpts::init();
    pseudofs::init();

    ext2::init();
    exfat::init();
    overlayfs::init();
}

pub(super) fn init_on_each_cpu() {
    procfs::init_on_each_cpu();
}
