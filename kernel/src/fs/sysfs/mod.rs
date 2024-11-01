// SPDX-License-Identifier: MPL-2.0
#![allow(unused)]

use core::sync::atomic::{AtomicU64, Ordering};

use devices::init_devices;
use inode::KObject;
use kernel::init_kernel;
use spin::Once;

use crate::{
    fs::{
        kernfs::{DataProvider, PseudoFileSystem, PseudoNode},
        utils::{FileSystem, FsFlags, Inode, SuperBlock, NAME_MAX},
    },
    prelude::*,
};

mod devices;
pub mod inode;
mod kernel;

/// SysFS filesystem.
/// Magic number.
const SYSFS_MAGIC: u64 = 0x62656572;
/// Root Inode ID.
const SYSFS_ROOT_INO: u64 = 1;
/// Block size.
const BLOCK_SIZE: usize = 1024;

pub struct SysFS {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    inode_allocator: AtomicU64,
    this: Weak<Self>,
}

impl SysFS {
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_fs| Self {
            sb: SuperBlock::new(SYSFS_MAGIC, BLOCK_SIZE, NAME_MAX),
            root: SysfsRoot::new_inode(weak_fs.clone()),
            inode_allocator: AtomicU64::new(SYSFS_ROOT_INO + 1),
            this: weak_fs.clone(),
        })
    }

    pub fn create_file(
        name: &str,
        mode: u16,
        parent: Arc<KObject>,
        attribute: Box<dyn DataProvider>,
    ) -> Result<Arc<KObject>> {
        let attr = KObject::new_attr(name, mode, Some(parent.this_weak()))?;
        attr.set_data(attribute).unwrap();
        Ok(attr)
    }

    pub fn create_kobject(name: &str, mode: u16, parent: Arc<KObject>) -> Result<Arc<KObject>> {
        KObject::new_dir(name, mode, Some(parent.this_weak()))
    }

    pub fn create_symlink(
        name: &str,
        parent: Arc<KObject>,
        target: Arc<dyn PseudoNode>,
    ) -> Result<Arc<KObject>> {
        KObject::new_link(name, Some(parent.this_weak()), target)
    }
}

impl PseudoFileSystem for SysFS {
    fn alloc_id(&self) -> u64 {
        self.inode_allocator.fetch_add(1, Ordering::SeqCst)
    }

    fn init(&self) -> Result<()> {
        let root = self.root_inode().downcast_ref::<KObject>().unwrap().this();
        let kernel_kobj = SysFS::create_kobject("kernel", 0o755, root.clone())?;
        init_kernel(kernel_kobj)?;
        let devices_kobj = SysFS::create_kobject("devices", 0o755, root.clone())?;
        init_devices(devices_kobj)?;
        let fs_kobj = SysFS::create_kobject("fs", 0o755, root.clone())?;
        SysFS::create_kobject("cgroup", 0o755, fs_kobj.clone())?;
        Ok(())
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.this.upgrade().unwrap()
    }
}

impl FileSystem for SysFS {
    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        self.sb.clone()
    }

    fn flags(&self) -> FsFlags {
        FsFlags::empty()
    }
}

/// Represents the inode at `/sys`.
/// Root directory of the sysfs.
pub struct SysfsRoot;

impl SysfsRoot {
    pub fn new_inode(fs: Weak<SysFS>) -> Arc<dyn Inode> {
        KObject::new_root("sysfs", fs, SYSFS_ROOT_INO, BLOCK_SIZE)
    }
}
