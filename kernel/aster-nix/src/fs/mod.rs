// SPDX-License-Identifier: MPL-2.0

pub mod device;
pub mod devpts;
pub mod epoll;
pub mod exfat;
pub mod ext2;
pub mod file_handle;
pub mod file_table;
pub mod fs_resolver;
pub mod inode_handle;
pub mod path;
pub mod pipe;
pub mod procfs;
pub mod ramfs;
pub mod rootfs;
pub mod utils;

use crate::{
    device::start_block_device,
    fs::{
        exfat::{ExfatFS, ExfatMountOptions},
        ext2::Ext2,
        fs_resolver::FsPath,
    },
    prelude::*,
};

pub fn lazy_init() {
    // Following device names are specified in qemu args as `--serial={device_name}`
    const EXT2_DEVICE_NAME: &str = "vext2";
    const EXFAT_DEVICE_NAME: &str = "vexfat";

    if let Ok(block_device_ext2) = start_block_device(EXT2_DEVICE_NAME) {
        let ext2_fs = Ext2::open(block_device_ext2).unwrap();
        let target_path = FsPath::try_from("/ext2").unwrap();
        println!("[kernel] Mount Ext2 fs at {:?} ", target_path);
        self::rootfs::mount_fs_at(ext2_fs, &target_path).unwrap();
    }

    if let Ok(block_device_exfat) = start_block_device(EXFAT_DEVICE_NAME) {
        let exfat_fs = ExfatFS::open(block_device_exfat, ExfatMountOptions::default()).unwrap();
        let target_path = FsPath::try_from("/exfat").unwrap();
        println!("[kernel] Mount ExFat fs at {:?} ", target_path);
        self::rootfs::mount_fs_at(exfat_fs, &target_path).unwrap();
    }
}
