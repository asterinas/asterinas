// SPDX-License-Identifier: MPL-2.0

//! Concrete file system implementations.
//!
//! This module contains all the specific file system implementations supported by the kernel.

pub(crate) mod cgroupfs;
pub(crate) mod configfs;
pub(crate) mod devpts;
pub(crate) mod exfat;
pub(crate) mod ext2;
pub(crate) mod overlayfs;
pub(crate) mod procfs;
pub(crate) mod pseudofs;
pub(crate) mod ramfs;
pub(crate) mod sysfs;
pub(crate) mod tmpfs;
pub(crate) mod virtiofs;

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
    virtiofs::init();
}

pub(super) fn init_on_each_cpu() {
    procfs::init_on_each_cpu();
}
