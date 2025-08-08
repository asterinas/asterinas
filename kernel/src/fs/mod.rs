// SPDX-License-Identifier: MPL-2.0

pub mod cgroupfs;
pub mod device;
pub mod devpts;
pub mod epoll;
pub mod exfat;
pub mod ext2;
pub mod file_handle;
pub mod file_table;
pub mod fs_resolver;
pub mod inode_handle;
pub mod named_pipe;
pub mod overlayfs;
pub mod path;
pub mod pipe;
pub mod procfs;
pub mod ramfs;
pub mod registry;
pub mod rootfs;
pub mod sysfs;
pub mod thread_info;
pub mod utils;

pub fn lazy_init() {
    registry::init();

    sysfs::init();
    procfs::init();
    cgroupfs::init();
    ramfs::init();
    devpts::init();

    ext2::init();
    exfat::init();
    overlayfs::init();

    device::init();
}
