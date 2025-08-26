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

use aster_block::BlockDevice;
use aster_virtio::device::block::device::BlockDevice as VirtIoBlockDevice;

use crate::{
    fs::{
        exfat::{ExfatFS, ExfatMountOptions},
        ext2::Ext2,
        file_table::FdFlags,
        fs_resolver::{FsPath, FsResolver},
        utils::{AccessMode, InodeMode},
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

pub fn init() {
    registry::init();

    sysfs::init();
    procfs::init();
    cgroupfs::init();
    ramfs::init();
    devpts::init();

    ext2::init();
    exfat::init();
    overlayfs::init();
}

pub fn init_in_first_kthread(fs_resolver: &FsResolver) {
    rootfs::init_in_first_kthread(fs_resolver).unwrap();
}

pub fn init_in_first_process(ctx: &Context) {
    //The device name is specified in qemu args as --serial={device_name}
    let ext2_device_name = "vext2";
    let exfat_device_name = "vexfat";

    let fs = ctx.thread_local.borrow_fs();
    let fs_resolver = fs.resolver().read();

    if let Ok(block_device_ext2) = start_block_device(ext2_device_name) {
        let ext2_fs = Ext2::open(block_device_ext2).unwrap();
        let target_path = FsPath::try_from("/ext2").unwrap();
        println!("[kernel] Mount Ext2 fs at {:?} ", target_path);
        self::rootfs::mount_fs_at(ext2_fs, &target_path, &fs_resolver, ctx).unwrap();
    }

    if let Ok(block_device_exfat) = start_block_device(exfat_device_name) {
        let exfat_fs = ExfatFS::open(block_device_exfat, ExfatMountOptions::default()).unwrap();
        let target_path = FsPath::try_from("/exfat").unwrap();
        println!("[kernel] Mount ExFat fs at {:?} ", target_path);
        self::rootfs::mount_fs_at(exfat_fs, &target_path, &fs_resolver, ctx).unwrap();
    }

    // Initialize the file table for the first process.
    let tty_path = FsPath::new(fs_resolver::AT_FDCWD, "/dev/console").expect("cannot find tty");
    let stdin = {
        let flags = AccessMode::O_RDONLY as u32;
        let mode = InodeMode::S_IRUSR;
        fs_resolver.open(&tty_path, flags, mode.bits()).unwrap()
    };
    let stdout = {
        let flags = AccessMode::O_WRONLY as u32;
        let mode = InodeMode::S_IWUSR;
        fs_resolver.open(&tty_path, flags, mode.bits()).unwrap()
    };
    let stderr = {
        let flags = AccessMode::O_WRONLY as u32;
        let mode = InodeMode::S_IWUSR;
        fs_resolver.open(&tty_path, flags, mode.bits()).unwrap()
    };

    let mut file_table_ref = ctx.thread_local.borrow_file_table_mut();
    let mut file_table = file_table_ref.unwrap().write();

    file_table.insert(Arc::new(stdin), FdFlags::empty());
    file_table.insert(Arc::new(stdout), FdFlags::empty());
    file_table.insert(Arc::new(stderr), FdFlags::empty());
}
