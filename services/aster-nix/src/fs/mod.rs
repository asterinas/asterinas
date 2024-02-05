// SPDX-License-Identifier: MPL-2.0

pub mod device;
pub mod devpts;
pub mod epoll;
pub mod ext2;
pub mod file_handle;
pub mod file_table;
pub mod fs_resolver;
pub mod inode_handle;
pub mod pipe;
pub mod procfs;
pub mod ramfs;
pub mod rootfs;
pub mod utils;

use crate::fs::{ext2::Ext2, fs_resolver::FsPath};
use crate::prelude::*;
use crate::thread::kernel_thread::KernelThreadExt;
use aster_virtio::device::block::device::BlockDevice as VirtIoBlockDevice;
use aster_virtio::device::block::DEVICE_NAME as VIRTIO_BLOCK_NAME;

pub fn lazy_init() {
    let block_device = aster_block::get_device(VIRTIO_BLOCK_NAME).unwrap();
    let cloned_block_device = block_device.clone();

    let task_fn = move || {
        info!("spawn the virt-io-block thread");
        let virtio_block_device = block_device.downcast_ref::<VirtIoBlockDevice>().unwrap();
        loop {
            virtio_block_device.handle_requests();
        }
    };
    crate::Thread::spawn_kernel_thread(crate::ThreadOptions::new(task_fn));

    let ext2_fs = Ext2::open(cloned_block_device).unwrap();
    let target_path = FsPath::try_from("/ext2").unwrap();
    println!("[kernel] Mount Ext2 fs at {:?} ", target_path);
    self::rootfs::mount_fs_at(ext2_fs, &target_path).unwrap();
}
