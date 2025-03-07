// SPDX-License-Identifier: MPL-2.0
#![allow(unused)]

use core::sync::atomic::{AtomicU64, Ordering};

use devices::register_devices;
use event::UEvent;
use kernel::register_kernel;
use power::register_power;
use spin::Once;

use super::{
    kernfs::{EventHandler, PseudoINode},
    utils::InodeMode,
};
use crate::{
    fs::{
        kernfs::{DataProvider, PseudoFileSystem},
        utils::{FileSystem, FsFlags, Inode, SuperBlock, NAME_MAX},
    },
    prelude::*,
};

mod devices;
mod event;
mod kernel;
mod power;

/// SysFS filesystem.
/// Magic number.
const SYSFS_MAGIC: u64 = 0x62656572;
/// Root Inode ID.
const SYSFS_ROOT_INO: u64 = 1;
/// Block size.
const BLOCK_SIZE: usize = 1024;

pub type KObject = PseudoINode;

/// Represents the SysFS filesystem.
pub struct SysFS {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    inode_allocator: AtomicU64,
    this: Weak<Self>,
}

impl SysFS {
    pub fn new() -> Arc<Self> {
        let fs = Arc::new_cyclic(|weak_fs| Self {
            sb: SuperBlock::new(SYSFS_MAGIC, BLOCK_SIZE, NAME_MAX),
            root: SysfsRoot::new_inode(weak_fs.clone()),
            inode_allocator: AtomicU64::new(SYSFS_ROOT_INO + 1),
            this: weak_fs.clone(),
        });
        let root = fs.root_inode().downcast_ref::<KObject>().unwrap().this();
        register_kernel(root.clone()).unwrap();
        register_devices(root.clone()).unwrap();
        register_power(root.clone()).unwrap();
        fs
    }

    pub fn create_attribute(
        name: &str,
        mode: u16,
        parent: Arc<KObject>,
        attribute: Box<dyn DataProvider>,
        event_handler: Option<Arc<dyn EventHandler>>,
    ) -> Result<Arc<KObject>> {
        let mode = InodeMode::from_bits_truncate(mode);
        let attr = KObject::new_attr(name, Some(mode), event_handler, parent.this_weak())?;
        attr.set_data(attribute).unwrap();
        Ok(attr)
    }

    pub fn create_kobject(
        name: &str,
        mode: u16,
        parent: Arc<KObject>,
        event_handler: Option<Arc<dyn EventHandler>>,
    ) -> Result<Arc<KObject>> {
        let mode = InodeMode::from_bits_truncate(mode);
        KObject::new_dir(name, Some(mode), event_handler, parent.this_weak())
    }

    pub fn create_symlink(
        name: &str,
        parent: Arc<KObject>,
        target: &str,
        event_handler: Option<Arc<dyn EventHandler>>,
    ) -> Result<Arc<KObject>> {
        KObject::new_symlink(name, target, parent.this_weak(), event_handler)
    }
}

impl PseudoFileSystem for SysFS {
    fn alloc_id(&self) -> u64 {
        self.inode_allocator.fetch_add(1, Ordering::SeqCst)
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

/// Represents the root directory of the sysfs filesystem
pub struct SysfsRoot;

impl SysfsRoot {
    pub fn new_inode(fs: Weak<SysFS>) -> Arc<dyn Inode> {
        KObject::new_root(fs, SYSFS_ROOT_INO, BLOCK_SIZE, None)
    }
}
