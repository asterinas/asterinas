// SPDX-License-Identifier: MPL-2.0
#![allow(unused)]

use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
};
use core::time::Duration;

use ostd::sync::RwLock;

use super::{
    element::{DataProvider, KernfsElem},
    PseudoFileSystem, BLOCK_SIZE,
};
use crate::{
    events::IoEvents,
    fs::{
        device::Device,
        utils::{
            DirentVisitor, FallocMode, FileSystem, Inode, InodeMode, InodeType, IoctlCmd, Metadata,
            MknodType,
        },
        Errno,
    },
    prelude::*,
    process::{signal::Poller, Gid, Uid},
};

bitflags! {
    /// Some flags in Linux are used to indicate the status of the node. We don't need them.
    pub struct KernfsNodeFlag: u16 {
        /// Indicates that the node supports namespaces.
        const KERNFS_NS         = 0x0020;
        /// Indicates that the node supports the `seq_file` interface.
        const KERNFS_HAS_SEQ_SHOW = 0x0040;
        /// Indicates that the node supports the `mmap` operation.
        const KERNFS_HAS_MMAP   = 0x0080;
        /// Indicates that the node has lock dependency tracking enabled.
        const KERNFS_LOCKDEP    = 0x0100;
        /// Indicates that the node is hidden.
        const KERNFS_HIDDEN     = 0x0200;
    }
}

#[derive(Debug)]
struct Inner {
    name: String,
    flags: KernfsNodeFlag,
    metadata: Metadata,
    elem: KernfsElem,
    fs: Weak<dyn PseudoFileSystem>,
}

/// Represents a node in the kernel filesystem (kernfs).
///
/// This struct contains the core information and functionality for a kernfs node,
/// including its metadata, flags, and relationship to other nodes in the filesystem.
/// It is used to implement various types of filesystem entries like directories,
/// regular files, and special files in the kernel's pseudo-filesystem.
#[derive(Debug)]
pub struct KernfsNode {
    inner: RwLock<Inner>,
    parent: Option<Weak<KernfsNode>>,
    this: Weak<KernfsNode>,
}

impl KernfsNode {
    pub fn new_root(
        name: &str,
        fs: Weak<dyn PseudoFileSystem>,
        root_ino: u64,
        blk_size: usize,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            inner: RwLock::new(Inner {
                name: name.to_string(),
                flags: KernfsNodeFlag::empty(),
                metadata: Metadata::new_dir(
                    root_ino,
                    InodeMode::from_bits_truncate(0o555),
                    blk_size,
                ),
                elem: KernfsElem::new_dir(),
                fs,
            }),
            parent: None,
            this: weak_self.clone(),
        })
    }

    pub fn new_attr(
        name: &str,
        mode: Option<InodeMode>,
        flags: KernfsNodeFlag,
        parent: Weak<KernfsNode>,
    ) -> Result<Arc<Self>> {
        let arc_parent = parent.upgrade().unwrap();
        let ino = arc_parent.generate_ino();

        let mode = mode.unwrap_or_else(|| InodeMode::from_bits_truncate(0o777));
        let metadata = Metadata::new_file(ino, mode, BLOCK_SIZE);

        let new_inode = Arc::new_cyclic(|weak_self| Self {
            inner: RwLock::new(Inner {
                name: name.to_string(),
                flags,
                metadata,
                elem: KernfsElem::new_attr(),
                fs: Arc::downgrade(&arc_parent.pseudo_fs()),
            }),
            parent: Some(parent),
            this: weak_self.clone(),
        });

        arc_parent.insert(name.to_string(), new_inode.clone())?;

        Ok(new_inode)
    }

    pub fn new_dir(
        name: &str,
        mode: Option<InodeMode>,
        flags: KernfsNodeFlag,
        parent: Weak<KernfsNode>,
    ) -> Result<Arc<Self>> {
        let arc_parent = parent.upgrade().unwrap();
        let ino = arc_parent.generate_ino();
        let mode = mode.unwrap_or(arc_parent.metadata().mode);
        let metadata = Metadata::new_dir(ino, mode, BLOCK_SIZE);

        let new_inode = Arc::new_cyclic(|weak_self| Self {
            inner: RwLock::new(Inner {
                name: name.to_string(),
                flags: KernfsNodeFlag::empty(),
                metadata,
                elem: KernfsElem::new_dir(),
                fs: Arc::downgrade(&arc_parent.pseudo_fs()),
            }),
            parent: Some(parent),
            this: weak_self.clone(),
        });
        arc_parent.insert(name.to_string(), new_inode.clone())?;
        Ok(new_inode)
    }

    pub fn new_symlink(
        name: &str,
        flags: KernfsNodeFlag,
        target: Weak<dyn Inode>,
        parent: Weak<KernfsNode>,
    ) -> Result<Arc<Self>> {
        let arc_parent = parent.upgrade().unwrap();
        let ino = arc_parent.generate_ino();
        let mode = InodeMode::from_bits_truncate(0o777);
        let metadata = Metadata::new_symlink(ino, mode, BLOCK_SIZE);

        let new_inode = Arc::new_cyclic(|weak_self| Self {
            inner: RwLock::new(Inner {
                name: name.to_string(),
                flags: KernfsNodeFlag::empty(),
                metadata,
                elem: KernfsElem::new_symlink(target),
                fs: Arc::downgrade(&arc_parent.pseudo_fs()),
            }),
            parent: Some(parent),
            this: weak_self.clone(),
        });
        arc_parent.insert(name.to_string(), new_inode.clone())?;
        Ok(new_inode)
    }
}

impl KernfsNode {
    pub fn name(&self) -> String {
        self.inner.read().name.clone()
    }

    /// Get the parent of the current node.
    pub fn parent(&self) -> Option<Arc<KernfsNode>> {
        self.parent.as_ref().and_then(|p| p.upgrade())
    }

    /// Get the current node.
    pub fn this(&self) -> Arc<KernfsNode> {
        self.this.upgrade().unwrap()
    }

    /// Get the current node as a weak reference.
    pub fn this_weak(&self) -> Weak<KernfsNode> {
        self.this.clone()
    }

    /// Get the pseudo filesystem of the current node.
    pub fn pseudo_fs(&self) -> Arc<dyn PseudoFileSystem> {
        self.inner.read().fs.upgrade().unwrap()
    }

    /// Generate a new inode number for the current node.
    pub fn generate_ino(&self) -> u64 {
        self.pseudo_fs().alloc_id()
    }

    /// Set the data of the current node.
    pub fn set_data(&self, data: Box<dyn DataProvider>) -> Result<()> {
        self.inner.write().elem.set_data(data)
    }

    /// Remove a child from the current node.
    /// Return Ok(()) if the child is removed successfully.
    /// Current node should be a directory.
    fn remove(&self, name: &str) -> Result<()> {
        self.inner.write().elem.remove(name)
    }

    /// Insert a child to the current node.
    /// Return Ok(()) if the child is inserted successfully.
    /// Current node should be a directory.
    /// The child should not exist in the current node.
    fn insert(&self, name: String, node: Arc<dyn Inode>) -> Result<()> {
        self.inner.write().elem.insert(name, node)
    }

    /// Get the children of the current node.
    fn get_children(&self) -> Option<BTreeMap<String, Arc<dyn Inode>>> {
        self.inner.read().elem.get_children()
    }
}

impl Drop for KernfsNode {
    /// Drop the current node.
    /// Remove the current node from the parent node.
    fn drop(&mut self) {
        let parent = if let Some(parent) = self.parent() {
            parent
        } else {
            return;
        };

        let _ = parent.remove(&self.inner.read().name);
    }
}

impl Inode for KernfsNode {
    fn type_(&self) -> InodeType {
        self.metadata().type_
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        self.inner.write().metadata.size = new_size;
        // FIXME: The resize operation should be supported for regular files.
        // A possible implementation is adding a size() for DataProvider.
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        self.inner.read().metadata
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
        self.inner.write().metadata.atime = time;
    }

    fn mtime(&self) -> Duration {
        self.metadata().mtime
    }

    fn set_mtime(&self, time: Duration) {
        self.inner.write().metadata.mtime = time;
    }

    fn ctime(&self) -> Duration {
        self.metadata().ctime
    }

    fn set_ctime(&self, time: Duration) {
        self.inner.write().metadata.ctime = time;
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.inner.read().fs.upgrade().unwrap()
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.inner.write().metadata.mode = mode;
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        self.inner.write().metadata.uid = uid;
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        self.inner.write().metadata.gid = gid;
        Ok(())
    }

    fn page_cache(&self) -> Option<crate::vm::vmo::Vmo<aster_rights::Full>> {
        None
    }

    fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        self.inner.read().elem.read_at(offset, buf)
    }

    fn read_direct_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        self.read_at(offset, buf)
    }

    fn write_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        debug!("KernfsNode::write_at is called");

        self.inner.write().elem.write_at(offset, buf)
    }

    fn write_direct_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        self.write_at(offset, buf)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self.lookup(name).is_ok() {
            return_errno!(Errno::EEXIST);
        }
        let new_node = match type_ {
            InodeType::Dir => {
                KernfsNode::new_dir(name, Some(mode), KernfsNodeFlag::empty(), self.this_weak())
            }
            InodeType::File => {
                KernfsNode::new_attr(name, Some(mode), KernfsNodeFlag::empty(), self.this_weak())
            }
            InodeType::SymLink => KernfsNode::new_symlink(
                name,
                KernfsNodeFlag::empty(),
                self.this_weak(),
                self.this_weak(),
            ),
            _ => return_errno!(Errno::EINVAL),
        }?;
        Ok(new_node)
    }

    fn mknod(&self, name: &str, mode: InodeMode, dev: MknodType) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn as_device(&self) -> Option<Arc<dyn Device>> {
        None
    }

    fn readdir_at(&self, mut offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            // Read the two special entries.
            if *offset == 0 {
                let this_inode = self.this();
                visitor.visit(".", this_inode.ino(), this_inode.type_(), *offset)?;
                *offset += 1;
            }
            if *offset == 1 {
                let parent_inode = self.parent().unwrap_or(self.this());
                visitor.visit("..", parent_inode.ino(), parent_inode.type_(), *offset)?;
                *offset += 1;
            }

            // Read the normal child entries.
            let cached_children = self.get_children().unwrap();
            let start_idx = *offset;

            for (name, inode) in cached_children.iter().skip(*offset - 2) {
                visitor.visit(name, inode.ino(), inode.type_(), *offset - 2)?;
                *offset += 1;
            }

            Ok(())
        };
        try_readdir(&mut offset, visitor)?;
        Ok(offset)
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        // Create a hard link to the old inode.
        // The old inode should be a regular file or a directory.
        // The name should not be "." or "..".
        // The name should not exist in the current directory.
        if old.type_() != InodeType::File && old.type_() != InodeType::Dir {
            return_errno!(Errno::EPERM);
        }
        if name == "." || name == ".." {
            return_errno!(Errno::EPERM);
        }
        if self.lookup(name).is_ok() {
            return_errno!(Errno::EEXIST);
        }
        let target = old
            .downcast_ref::<KernfsNode>()
            .ok_or(Error::new(Errno::EXDEV))?
            .this_weak();
        let new_node =
            KernfsNode::new_symlink(name, KernfsNodeFlag::empty(), target, self.this_weak())?;
        Ok(())
    }

    fn unlink(&self, name: &str) -> Result<()> {
        // Remove a child from the current node.
        // The child should be a regular file or a directory.
        // The child should not be "." or "..".
        if name == "." || name == ".." {
            return_errno!(Errno::EPERM);
        }
        if self.lookup(name)?.type_() != InodeType::SymLink {
            return_errno!(Errno::EPERM);
        }
        self.remove(name)
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        // Remove a child from the current node.
        // The child should be a directory.
        // The child should not be "." or "..".
        if name == "." || name == ".." {
            return_errno!(Errno::EPERM);
        }
        if self.lookup(name)?.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        self.remove(name)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = match name {
            "." => self.this(),
            ".." => self.parent().unwrap_or(self.this()),
            name => self.inner.read().elem.lookup(name)?,
        };
        Ok(inode)
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "rename is not supported in pseudo filesystem");
    }

    fn read_link(&self) -> Result<String> {
        if self.type_() != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        Ok(self.inner.read().name.clone())
    }

    fn write_link(&self, target: &str) -> Result<()> {
        if self.type_() != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        self.inner.write().name = target.to_string();
        Ok(())
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        Err(Error::new(Errno::EISDIR))
    }

    fn sync_all(&self) -> Result<()> {
        Ok(())
    }

    fn sync_data(&self) -> Result<()> {
        Ok(())
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        return_errno!(Errno::EOPNOTSUPP);
    }

    fn poll(&self, mask: IoEvents, _poller: Option<&mut Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }

    fn is_dentry_cacheable(&self) -> bool {
        true
    }
}
