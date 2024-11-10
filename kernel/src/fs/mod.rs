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
pub mod named_pipe;
pub mod path;
pub mod pipe;
pub mod procfs;
pub mod ramfs;
pub mod rootfs;
pub mod utils;

use aster_block::BlockDevice;
use aster_virtio::device::block::device::BlockDevice as VirtIoBlockDevice;

use crate::{
    fs::{
        exfat::{ExfatFS, ExfatMountOptions},
        ext2::Ext2,
        fs_resolver::FsPath,
    },
    prelude::*,
};

fn start_block_device(device_name: &str) -> Result<Arc<dyn BlockDevice>> {
    if let Some(device) = aster_block::get_device(device_name) {
        let cloned_device = device.clone();
        let task_fn = move || {
            info!("spawn the virt-io-block thread");
            let virtio_block_device = cloned_device.downcast_ref::<VirtIoBlockDevice>().unwrap();
            loop {
                virtio_block_device.handle_requests();
            }
        };
        crate::ThreadOptions::new(task_fn).spawn();
        Ok(device)
    } else {
        return_errno_with_message!(Errno::ENOENT, "Device does not exist")
    }
}

pub fn lazy_init() {
    //The device name is specified in qemu args as --serial={device_name}
    let ext2_device_name = "vext2";
    let exfat_device_name = "vexfat";

    if let Ok(block_device_ext2) = start_block_device(ext2_device_name) {
        let ext2_fs = Ext2::open(block_device_ext2).unwrap();
        let target_path = FsPath::try_from("/ext2").unwrap();
        println!("[kernel] Mount Ext2 fs at {:?} ", target_path);
        self::rootfs::mount_fs_at(ext2_fs, &target_path).unwrap();
    }

    if let Ok(block_device_exfat) = start_block_device(exfat_device_name) {
        let exfat_fs = ExfatFS::open(block_device_exfat, ExfatMountOptions::default()).unwrap();
        let target_path = FsPath::try_from("/exfat").unwrap();
        println!("[kernel] Mount ExFat fs at {:?} ", target_path);
        self::rootfs::mount_fs_at(exfat_fs, &target_path).unwrap();
    }
}
