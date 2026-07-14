// SPDX-License-Identifier: MPL-2.0

//! Concrete file system implementations.
//!
//! This module contains all the specific file system implementations supported by the kernel.

pub mod cgroupfs;
pub mod configfs;
pub mod devpts;
pub mod devtmpfs;
pub mod exfat;
pub mod ext2;
pub mod overlayfs;
pub mod procfs;
pub mod pseudofs;
pub mod ramfs;
pub mod sysfs;
pub mod tmpfs;
pub mod virtiofs;

pub(super) fn init() {
    sysfs::init();
    procfs::init();
    cgroupfs::init();
    configfs::init();
    ramfs::init();
    tmpfs::init();
    devtmpfs::init();
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

pub(super) fn init_in_first_kthread() {
    devtmpfs::init_in_first_kthread();
}
