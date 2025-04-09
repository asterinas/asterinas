// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    string::String,
    sync::{Arc, Weak},
};
use core::time::Duration;

use ostd::sync::RwLock;

use super::{
    element::CgroupElement, // Use renamed element module and type
    interfaces::{CgroupExt, DataProvider, BLOCK_SIZE_KERNFS}, // Use renamed traits module and type
    CgroupFs,
};
use crate::{
    fs::utils::{DirentVisitor, FileSystem, Inode, InodeMode, InodeType, Metadata, NAME_MAX},
    prelude::*,
    process::{Gid, Uid},
    time::clocks::RealTimeCoarseClock,
};

/// Represents a node in the cgroup filesystem (originally kernfs, now part of cgroupfs).
///
/// This struct contains the core information and functionality for an inode in the cgroup filesystem,
/// including its metadata, data, and relationship to other nodes.
pub struct CgroupINode {
    metadata: RwLock<Metadata>,            // Inode attributes
    elem: RwMutex<CgroupElement>,          // Concrete node type (from element.rs)
    extension: Option<Arc<dyn CgroupExt>>, // Extension hooks (trait from traits.rs) - Renamed field
    fs: Weak<CgroupFs>,                    // Concrete CgroupFs type
    parent: Option<Weak<CgroupINode>>,     // Parent directory (Self type)
    this: Weak<CgroupINode>,               // Self reference
}

impl CgroupINode {
    pub fn new_root(
        fs: Weak<CgroupFs>,
        root_ino: u64,
        blk_size: usize,
        extension: Option<Arc<dyn CgroupExt>>, // Use renamed trait and field name
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            metadata: RwLock::new(Metadata::new_dir(
                root_ino,
                InodeMode::from_bits_truncate(0o555),
                blk_size,
            )),
            elem: RwMutex::new(CgroupElement::new_dir()), // Use renamed type
            extension,                                    // Use renamed field
            fs,
            parent: None, // Root has no parent
            this: weak_self.clone(),
        })
    }

    pub fn new_attr(
        name: &str,
        mode: Option<InodeMode>,
        data: Option<Box<dyn DataProvider>>,
        extension: Option<Arc<dyn CgroupExt>>, // Use renamed trait and field name
        parent: Weak<CgroupINode>,             // Use self type
    ) -> Result<Arc<Self>> {
        let arc_parent = parent.upgrade().ok_or_else(|| Error::new(Errno::EINVAL))?; // Handle potential upgrade failure
        let ino = arc_parent.generate_ino();

        let mode = mode.unwrap_or_else(|| InodeMode::from_bits_truncate(0o644));
        let metadata = Metadata::new_file(ino, mode, BLOCK_SIZE_KERNFS); // Use original kernfs block size

        let new_inode = Arc::new_cyclic(|weak_self| Self {
            metadata: RwLock::new(metadata),
            elem: RwMutex::new(CgroupElement::new_attr(data)), // Use renamed type
            extension,                                         // Use renamed field
            fs: Arc::downgrade(&arc_parent.cgroup_fs()),
            parent: Some(parent), // Pass parent weak ref
            this: weak_self.clone(),
        });
        arc_parent.insert(name.to_string(), new_inode.clone())?;
        Ok(new_inode)
    }

    pub fn new_dir(
        name: &str,
        mode: Option<InodeMode>,
        extension: Option<Arc<dyn CgroupExt>>, // Use renamed trait and field name
        parent: Weak<CgroupINode>,             // Use self type
    ) -> Result<Arc<Self>> {
        let arc_parent = parent.upgrade().ok_or_else(|| Error::new(Errno::EINVAL))?; // Handle potential upgrade failure
        let ino = arc_parent.generate_ino();
        let mode = mode.unwrap_or(InodeMode::from_bits_truncate(0o755));
        let metadata = Metadata::new_dir(ino, mode, BLOCK_SIZE_KERNFS); // Use original kernfs block size

        let new_inode = Arc::new_cyclic(|weak_self| Self {
            metadata: RwLock::new(metadata),
            elem: RwMutex::new(CgroupElement::new_dir()), // Use renamed type
            extension,                                    // Use renamed field
            fs: Arc::downgrade(&arc_parent.cgroup_fs()),
            parent: Some(parent), // Pass parent weak ref
            this: weak_self.clone(),
        });
        arc_parent.insert(name.to_string(), new_inode.clone())?;
        Ok(new_inode)
    }

    pub fn new_symlink(
        name: &str,
        target: &str,
        extension: Option<Arc<dyn CgroupExt>>, // Use renamed trait and field name
        parent: Weak<CgroupINode>,             // Use self type
    ) -> Result<Arc<Self>> {
        let arc_parent = parent.upgrade().ok_or_else(|| Error::new(Errno::EINVAL))?; // Handle potential upgrade failure
        let ino = arc_parent.generate_ino();
        let mode = InodeMode::from_bits_truncate(0o777);
        let metadata = Metadata::new_symlink(ino, mode, BLOCK_SIZE_KERNFS); // Use original kernfs block size

        let new_inode = Arc::new_cyclic(|weak_self| Self {
            metadata: RwLock::new(metadata),
            elem: RwMutex::new(CgroupElement::new_symlink(target)), // Use renamed type
            extension,                                              // Use renamed field
            fs: Arc::downgrade(&arc_parent.cgroup_fs()),
            parent: Some(parent), // Pass parent weak ref
            this: weak_self.clone(),
        });
        arc_parent.insert(name.to_string(), new_inode.clone())?;
        Ok(new_inode)
    }

    /// Gets the current node.
    pub fn this(&self) -> Arc<CgroupINode> {
        self.this
            .upgrade()
            .expect("CgroupINode self reference should always be valid")
    }

    /// Gets the current node as a weak reference.
    pub fn this_weak(&self) -> Weak<CgroupINode> {
        self.this.clone()
    }
    /// Gets the parent of the current node.
    fn parent(&self) -> Option<Arc<CgroupINode>> {
        self.parent.as_ref().and_then(|p| p.upgrade())
    }

    /// Gets the concrete CgroupFs filesystem of the current node.
    fn cgroup_fs(&self) -> Arc<CgroupFs> {
        self.fs
            .upgrade()
            .expect("CgroupINode fs reference to CgroupFs should always be valid")
    }

    /// Generates a new inode number by calling the method on the concrete CgroupFs.
    fn generate_ino(&self) -> u64 {
        self.cgroup_fs().alloc_unique_id()
    }

    /// Sets the data of the current node.
    pub fn set_data(&self, data: Box<dyn DataProvider>) -> Result<()> {
        self.elem.write().set_data(data)
    }

    /// Sets the extension hooks for the current node.
    pub fn set_extension(&mut self, extension: Arc<dyn CgroupExt>) {
        // Use renamed trait and field name
        self.extension = Some(extension);
    }

    /// Removes a child from the current node.
    fn remove(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if let Some(extension) = self.extension.as_ref() {
            // Use renamed field
            extension.on_remove(name)?;
        }
        self.elem.write().remove(name)
    }

    /// Inserts a child to the current node.
    fn insert(&self, name: String, node: Arc<dyn Inode>) -> Result<()> {
        if let Some(extension) = self.extension.as_ref() {
            // Use renamed field
            extension.on_create(name.as_str(), node.clone())?;
        }
        self.elem.write().insert(name, node)
    }

    /// Reads data from the current node at the specified offset.
    fn cgroup_read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        // Renamed method
        self.elem.read().read_at(offset, buf)
    }

    /// Writes data to the current node at the specified offset.
    fn cgroup_write_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        // Renamed method
        self.elem.write().write_at(offset, buf)
    }
}

impl Inode for CgroupINode {
    fn type_(&self) -> InodeType {
        self.metadata().type_
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        *self.metadata.read()
    }

    fn ino(&self) -> u64 {
        self.metadata().ino
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata().mode)
    }

    fn size(&self) -> usize {
        self.metadata().size
    }

    fn atime(&self) -> Duration {
        self.metadata().atime
    }

    fn set_atime(&self, time: Duration) {
        self.metadata.write().atime = time;
    }

    fn mtime(&self) -> Duration {
        self.metadata().mtime
    }

    fn set_mtime(&self, time: Duration) {
        self.metadata.write().mtime = time;
    }

    fn ctime(&self) -> Duration {
        self.metadata().ctime
    }

    fn set_ctime(&self, time: Duration) {
        self.metadata.write().ctime = time;
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        // Upgrade to concrete type, then cast back to trait object
        self.cgroup_fs() as Arc<dyn FileSystem>
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.metadata.write().mode = mode;
        self.set_ctime(RealTimeCoarseClock::get().read_time());
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        self.metadata.write().uid = uid;
        self.set_ctime(RealTimeCoarseClock::get().read_time());
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        self.metadata.write().gid = gid;
        self.set_ctime(RealTimeCoarseClock::get().read_time());
        Ok(())
    }

    fn page_cache(&self) -> Option<crate::vm::vmo::Vmo<aster_rights::Full>> {
        None
    }

    fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        let size = self.cgroup_read_at(offset, buf)?; // Use renamed method
        if size > 0 {
            self.set_atime(RealTimeCoarseClock::get().read_time());
        }
        Ok(size)
    }

    fn read_direct_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        self.read_at(offset, buf)
    }

    fn write_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        let size = self.cgroup_write_at(offset, buf)?; // Use renamed method
        if size > 0 {
            let now = RealTimeCoarseClock::get().read_time();
            let mut meta = self.metadata.write();
            meta.mtime = now;
            meta.ctime = now;
            meta.size = meta.size.max(offset + size);
        }
        Ok(size)
    }

    fn write_direct_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        self.write_at(offset, buf)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self.lookup(name).is_ok() {
            return_errno!(Errno::EEXIST);
        }

        let new_node: Arc<CgroupINode> = match type_ {
            InodeType::Dir => CgroupINode::new_dir(
                name,
                Some(mode),
                self.extension.clone(), // Use renamed field
                self.this_weak(),
            )?,
            InodeType::File => CgroupINode::new_attr(
                name,
                Some(mode),
                None,
                self.extension.clone(), // Use renamed field
                self.this_weak(),
            )?,
            InodeType::SymLink => CgroupINode::new_symlink(
                name,
                "",
                self.extension.clone(), // Use renamed field
                self.this_weak(),
            )?,
            _ => return_errno!(Errno::EINVAL),
        };

        let now = RealTimeCoarseClock::get().read_time();
        self.set_mtime(now);
        self.set_ctime(now);

        Ok(new_node as Arc<dyn Inode>)
    }

    fn readdir_at(&self, mut offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let start_offset = offset;

        if offset == 0 {
            let this_inode = self.this();
            if visitor
                .visit(".", this_inode.ino(), this_inode.type_(), offset)
                .is_err()
            {
                return Ok(offset);
            }
            offset += 1;
        }
        if offset == 1 {
            let parent_inode = self.parent().unwrap_or_else(|| self.this());
            if visitor
                .visit("..", parent_inode.ino(), parent_inode.type_(), offset)
                .is_err()
            {
                return Ok(offset);
            }
            offset += 1;
        }

        let elem_read = self.elem.read();
        if let Some(children) = elem_read.get_children() {
            for (name, inode) in children.iter().skip(offset - 2) {
                if visitor
                    .visit(name, inode.ino(), inode.type_(), offset)
                    .is_err()
                {
                    return Ok(offset);
                }
                offset += 1;
            }
        } else {
            return_errno!(Errno::ENOTDIR);
        }

        if offset > start_offset {
            self.set_atime(RealTimeCoarseClock::get().read_time());
        }
        Ok(offset)
    }

    fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "hard links not supported in cgroupfs");
    }

    fn unlink(&self, name: &str) -> Result<()> {
        if name == "." || name == ".." {
            return_errno!(Errno::EINVAL);
        }
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let child = self.lookup(name)?;
        if child.type_() == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }

        self.remove(name)?;

        let now = RealTimeCoarseClock::get().read_time();
        self.set_mtime(now);
        self.set_ctime(now);
        Ok(())
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        if name == "." || name == ".." {
            return_errno!(Errno::EINVAL);
        }
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let child = self.lookup(name)?;
        if child.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let child_cgroup = child // Use renamed variable for clarity
            .downcast_ref::<CgroupINode>()
            .ok_or_else(|| Error::new(Errno::EIO))?;
        if child_cgroup // Use renamed variable
            .elem
            .read()
            .get_children() // Assumes get_children is still appropriate for CgroupElement::Dir
            .map_or(false, |c| !c.is_empty())
        {
            return_errno!(Errno::ENOTEMPTY);
        }

        self.remove(name)?;

        let now = RealTimeCoarseClock::get().read_time();
        self.set_mtime(now);
        self.set_ctime(now);
        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let inode: Arc<dyn Inode> = match name {
            "." => self.this(),
            ".." => self.parent().unwrap_or_else(|| self.this()),
            _ => self.elem.read().lookup(name)?,
        };
        Ok(inode)
    }

    fn rename(&self, _old_name: &str, _target: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "rename is not supported in cgroupfs");
    }

    fn read_link(&self) -> Result<String> {
        if self.type_() != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        let link_target = self.elem.read().read_link()?;
        self.set_atime(RealTimeCoarseClock::get().read_time());
        Ok(link_target)
    }

    fn write_link(&self, target: &str) -> Result<()> {
        if self.type_() != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        self.elem.write().write_link(target)?;
        let now = RealTimeCoarseClock::get().read_time();
        self.set_mtime(now);
        self.set_ctime(now);
        Ok(())
    }
}
