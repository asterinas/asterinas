// SPDX-License-Identifier: MPL-2.0
#![allow(unused)]

use core::sync::atomic::{AtomicU64, Ordering};

use devices::register_cpu_online;
use event::UEvent;
use kernel::register_huge_page;
use power::register_power_state;
use spin::Once;

use super::{
    kernfs::{PseudoExt, PseudoINode},
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

/// Magic number.
const SYSFS_MAGIC: u64 = 0x62656572;
/// Root Inode ID.
const SYSFS_ROOT_INO: u64 = 1;
/// Block size.
const BLOCK_SIZE: usize = 1024;

pub type KObject = PseudoINode;

/// The reference to the SysFS filesystem.
/// The devices can use this reference to register their attributes.
pub static SYSFS_REF: Once<Arc<SysFS>> = Once::new();

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
        SYSFS_REF.call_once(|| fs.clone());
        register_huge_page().unwrap();
        register_cpu_online().unwrap();
        register_power_state().unwrap();
        fs
    }

    pub fn create_attribute(
        name: &str,
        mode: u16,
        parent: Arc<KObject>,
        attribute: Box<dyn DataProvider>,
        pseudo_extension: Option<Arc<dyn PseudoExt>>,
    ) -> Result<Arc<KObject>> {
        let mode = InodeMode::from_bits_truncate(mode);
        let attr = KObject::new_attr(name, Some(mode), pseudo_extension, parent.this_weak())?;
        attr.set_data(attribute).unwrap();
        Ok(attr)
    }

    pub fn create_dir(
        name: &str,
        mode: u16,
        parent: Arc<KObject>,
        pseudo_extension: Option<Arc<dyn PseudoExt>>,
    ) -> Result<Arc<KObject>> {
        let mode = InodeMode::from_bits_truncate(mode);
        KObject::new_dir(name, Some(mode), pseudo_extension, parent.this_weak())
    }

    pub fn create_symlink(
        name: &str,
        parent: Arc<KObject>,
        target: &str,
        pseudo_extension: Option<Arc<dyn PseudoExt>>,
    ) -> Result<Arc<KObject>> {
        KObject::new_symlink(name, target, parent.this_weak(), pseudo_extension)
    }

    /// Initialize the parent directories of the given path.
    /// If the parent directories do not exist, create them.
    /// Return the KObject reference of the last directory.
    /// The extensions are not set. The caller should set them by `KObject::set_pseudo_extensions` if needed.
    pub fn init_parent_dirs(&self, parent: &str) -> Result<Arc<KObject>> {
        let mut parent_node = self.root_inode().downcast_ref::<KObject>().unwrap().this();
        for dir in parent.split('/') {
            if dir.is_empty() || dir == "sys" {
                continue;
            }
            if let Ok(next) = parent_node.lookup(dir) {
                parent_node = next.downcast_ref::<KObject>().unwrap().this();
            } else {
                parent_node = SysFS::create_dir(dir, 0o755, parent_node.clone(), None)?;
            }
        }
        Ok(parent_node)
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
