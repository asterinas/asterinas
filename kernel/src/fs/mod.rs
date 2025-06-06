// SPDX-License-Identifier: MPL-2.0

pub mod device;
pub mod devpts;
pub mod epoll;
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
pub mod rootfs;
pub mod sysfs;
pub mod thread_info;
pub mod utils;

use alloc::collections::BTreeMap;

use aster_block::BlockDevice;
use aster_virtio::device::block::device::BlockDevice as VirtIoBlockDevice;
use ostd::early_print;
use spin::Once;

use crate::{
    fs::{ext2::Ext2, fs_resolver::FsPath},
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
    if let Some(registrars) = FILESYSTEM_REGISTRARS.get() {
        let locked = registrars.lock();
        println!("[kernel] Registered filesystems:");
        for (name, _) in locked.iter() {
            println!("  - {}", name);
        }
    } else {
        println!("[kernel] No filesystems registered yet");
    }
    if let Ok(block_device_exfat) = start_block_device(exfat_device_name) {
        let registrar_opt = FILESYSTEM_REGISTRARS
            .call_once(|| Mutex::new(BTreeMap::new()))
            .lock()
            .get("exfat")
            .cloned();

        match registrar_opt {
            Some(registrar) => {
                let exfat_fs = registrar.open(block_device_exfat).unwrap();
                let target_path = FsPath::try_from("/exfat").unwrap();
                println!("[kernel] Mount ExFat fs at {:?} ", target_path);
                self::rootfs::mount_fs_at(exfat_fs, &target_path).unwrap();
            }
            None => {
                println!("[kernel] ExFat registrar not found");
            }
        }
    }
}

pub trait FileSystemRegistrar: Sync + Send {
    fn name(&self) -> &'static str;
    fn open(
        &self,
        block_device: Arc<dyn BlockDevice>,
    ) -> Result<Arc<dyn crate::fs::utils::FileSystem>>;
}
static FILESYSTEM_REGISTRARS: Once<Mutex<BTreeMap<&'static str, Arc<dyn FileSystemRegistrar>>>> =
    Once::new();

pub fn register_fs_registrar(name: &'static str, registrar: Arc<dyn FileSystemRegistrar>) {
    FILESYSTEM_REGISTRARS
        .call_once(|| Mutex::new(BTreeMap::new()))
        .lock()
        .insert(name, registrar);
}

pub fn get_fs_registrar(name: &str) -> Option<Arc<dyn FileSystemRegistrar>> {
    FILESYSTEM_REGISTRARS
        .get()
        .and_then(|registrars| registrars.lock().get(name).cloned())
}

#[macro_export]
macro_rules! register_filesystem {
    ($fs_name:expr, $reg_type:ty) => {
        const _: () = {
            #[allow(non_upper_case_globals)]
            #[ctor::ctor]
            fn __fs_init() {
                crate::fs::register_fs_registrar($fs_name, Arc::new(<$reg_type>::default()));
            }
        };
    };
}
