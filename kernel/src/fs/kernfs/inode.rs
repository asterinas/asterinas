// SPDX-License-Identifier: MPL-2.0

use alloc::{
    string::String,
    sync::{Arc, Weak},
};
use core::time::Duration;

use ostd::sync::RwMutex;

use super::{element::PseudoElement, DataProvider, PseudoExt, PseudoFileSystem, BLOCK_SIZE};
use crate::{
    fs::{
        utils::{DirentVisitor, FileSystem, Inode, InodeMode, InodeType, Metadata, NAME_MAX},
        Errno,
    },
    prelude::*,
    process::{Gid, Uid},
    time::clocks::RealTimeCoarseClock,
};

/// Represents a node in the kernel filesystem (kernfs).
///
/// This struct contains the core information and functionality for a inode in the pseudo filesystem,
/// including its metadata, data, and relationship to other nodes.
/// It is used to implement various types of filesystem entries like directories, regular files, and special files.
/// The pseudo_extension field provides different abstractions for different pseudo-file system requirements.
pub struct PseudoINode {
    metadata: RwLock<Metadata>,                   // Inode attributes
    elem: RwMutex<PseudoElement>,                 // Concrete node type
    pseudo_extension: Option<Arc<dyn PseudoExt>>, // Extension hooks
    fs: Weak<dyn PseudoFileSystem>,               // FS reference
    parent: Option<Weak<PseudoINode>>,            // Parent directory
    this: Weak<PseudoINode>,                      // Self reference
}

impl PseudoINode {
    pub fn new_root(
        fs: Weak<dyn PseudoFileSystem>,
        root_ino: u64,
        blk_size: usize,
        pseudo_extension: Option<Arc<dyn PseudoExt>>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            metadata: RwLock::new(Metadata::new_dir(
                root_ino,
                InodeMode::from_bits_truncate(0o555),
                blk_size,
            )),
            elem: RwMutex::new(PseudoElement::new_dir()),
            pseudo_extension,
            fs,
            parent: None,
            this: weak_self.clone(),
        })
    }

    pub fn new_attr(
        name: &str,
        mode: Option<InodeMode>,
        data: Option<Box<dyn DataProvider>>,
        pseudo_extension: Option<Arc<dyn PseudoExt>>,
        parent: Weak<PseudoINode>,
    ) -> Result<Arc<Self>> {
        let arc_parent = parent.upgrade().unwrap();
        let ino = arc_parent.generate_ino();

        let mode = mode.unwrap_or_else(|| InodeMode::from_bits_truncate(0o777));
        let metadata = Metadata::new_file(ino, mode, BLOCK_SIZE);

        let new_inode = Arc::new_cyclic(|weak_self| Self {
            metadata: RwLock::new(metadata),
            elem: RwMutex::new(PseudoElement::new_attr(data)),
            pseudo_extension,
            fs: Arc::downgrade(&arc_parent.pseudo_fs()),
            parent: Some(parent),
            this: weak_self.clone(),
        });
        arc_parent.insert(name.to_string(), new_inode.clone())?;
        Ok(new_inode)
    }

    pub fn new_dir(
        name: &str,
        mode: Option<InodeMode>,
        pseudo_extension: Option<Arc<dyn PseudoExt>>,
        parent: Weak<PseudoINode>,
    ) -> Result<Arc<Self>> {
        let arc_parent = parent.upgrade().unwrap();
        let ino = arc_parent.generate_ino();
        let mode = mode.unwrap_or(arc_parent.metadata().mode);
        let metadata = Metadata::new_dir(ino, mode, BLOCK_SIZE);

        let new_inode = Arc::new_cyclic(|weak_self| Self {
            metadata: RwLock::new(metadata),
            elem: RwMutex::new(PseudoElement::new_dir()),
            pseudo_extension,
            fs: Arc::downgrade(&arc_parent.pseudo_fs()),
            parent: Some(parent),
            this: weak_self.clone(),
        });
        arc_parent.insert(name.to_string(), new_inode.clone())?;
        Ok(new_inode)
    }

    pub fn new_symlink(
        name: &str,
        target: &str,
        pseudo_extension: Option<Arc<dyn PseudoExt>>,
        parent: Weak<PseudoINode>,
    ) -> Result<Arc<Self>> {
        let arc_parent = parent.upgrade().unwrap();
        let ino = arc_parent.generate_ino();
        let mode = InodeMode::from_bits_truncate(0o777);
        let metadata = Metadata::new_symlink(ino, mode, BLOCK_SIZE);

        let new_inode = Arc::new_cyclic(|weak_self| Self {
            metadata: RwLock::new(metadata),
            elem: RwMutex::new(PseudoElement::new_symlink(target)),
            pseudo_extension,
            fs: Arc::downgrade(&arc_parent.pseudo_fs()),
            parent: Some(parent),
            this: weak_self.clone(),
        });
        arc_parent.insert(name.to_string(), new_inode.clone())?;
        Ok(new_inode)
    }

    /// Gets the current node.
    pub fn this(&self) -> Arc<PseudoINode> {
        self.this.upgrade().unwrap()
    }

    /// Gets the current node as a weak reference.
    pub fn this_weak(&self) -> Weak<PseudoINode> {
        self.this.clone()
    }

    /// Gets the parent of the current node.
    fn parent(&self) -> Option<Arc<PseudoINode>> {
        self.parent.as_ref().and_then(|p| p.upgrade())
    }

    /// Gets the pseudo filesystem of the current node.
    fn pseudo_fs(&self) -> Arc<dyn PseudoFileSystem> {
        self.fs.upgrade().unwrap()
    }

    /// Generates a new inode number for the current node.
    fn generate_ino(&self) -> u64 {
        self.pseudo_fs().alloc_unique_id()
    }

    /// Sets the data of the current node.
    pub fn set_data(&self, data: Box<dyn DataProvider>) -> Result<()> {
        self.elem.write().set_data(data)
    }

    /// Sets the pseudo_extension of the current node.
    /// The pseudo_extension provides different abstractions for different pseudo-file system requirements.
    pub fn set_pseudo_extension(&mut self, pseudo_extension: Arc<dyn PseudoExt>) {
        self.pseudo_extension = Some(pseudo_extension);
    }

    /// Removes a child from the current node.
    /// Returns Ok(Arc<dyn Inode>) if the child is removed successfully.
    /// Current node should be a directory.
    /// Triggers the pseudo_extension's `on_remove` method if the pseudo_extension exists.
    fn remove(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if let Some(pseudo_extension) = self.pseudo_extension.as_ref() {
            pseudo_extension.on_remove(name)?;
        }
        self.elem.write().remove(name)
    }

    /// Inserts a child to the current node.
    /// Returns Ok(()) if the child is inserted successfully.
    /// Current node should be a directory.
    /// The child should not exist in the current node.
    /// Triggers the pseudo_extension's `on_create` method if the pseudo_extension exists.
    fn insert(&self, name: String, node: Arc<dyn Inode>) -> Result<()> {
        if let Some(pseudo_extension) = self.pseudo_extension.as_ref() {
            pseudo_extension.on_create(name.as_str(), node.clone())?;
        }
        self.elem.write().insert(name, node)
    }

    /// Reads data from the current node at the specified offset.
    fn pseudo_read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        self.elem.read().read_at(offset, buf)
    }

    /// Writes data to the current node at the specified offset.
    fn pseudo_write_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        self.elem.write().write_at(offset, buf)
    }
}

impl Inode for PseudoINode {
    fn type_(&self) -> InodeType {
        self.metadata().type_
    }

    // File size in pseudo filesystem usually is equal to 0.
    fn resize(&self, new_size: usize) -> Result<()> {
        let mut elem = self.elem.write();
        if let PseudoElement::Attr(attr) = &mut *elem {
            attr.truncate(new_size)?;
        }
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
        self.fs.upgrade().unwrap()
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.metadata.write().mode = mode;
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        self.metadata.write().uid = uid;
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        self.metadata.write().gid = gid;
        Ok(())
    }

    fn page_cache(&self) -> Option<crate::vm::vmo::Vmo<aster_rights::Full>> {
        None
    }

    fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        let size = self.pseudo_read_at(offset, buf)?;
        self.set_atime(RealTimeCoarseClock::get().read_time());
        Ok(size)
    }

    fn read_direct_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        self.read_at(offset, buf)
    }

    fn write_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        let size = self.pseudo_write_at(offset, buf)?;
        self.set_mtime(RealTimeCoarseClock::get().read_time());
        self.set_ctime(RealTimeCoarseClock::get().read_time());
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
        let new_node = match type_ {
            InodeType::Dir => PseudoINode::new_dir(name, Some(mode), None, self.this_weak()),
            InodeType::File => {
                PseudoINode::new_attr(name, Some(mode), None, None, self.this_weak())
            }
            InodeType::SymLink => PseudoINode::new_symlink(name, "", None, self.this_weak()),
            _ => return_errno!(Errno::EINVAL),
        }?;
        Ok(new_node)
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
            let cached_children = self.elem.read().get_children().unwrap_or_default();

            for (name, inode) in cached_children.iter().skip(*offset - 2) {
                visitor.visit(name, inode.ino(), inode.type_(), *offset - 2)?;
                *offset += 1;
            }

            Ok(())
        };
        try_readdir(&mut offset, visitor)?;
        Ok(offset)
    }

    fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "hard links not supported in kernfs");
    }

    fn unlink(&self, name: &str) -> Result<()> {
        // Removes a child from the current node.
        // The child should be a regular file or a directory.
        // The child should not be "." or "..".
        if name == "." || name == ".." {
            return_errno!(Errno::EPERM);
        }
        if self.lookup(name)?.type_() == InodeType::Dir {
            return_errno!(Errno::EPERM);
        }
        self.remove(name)?;
        Ok(())
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        // Removes a child from the current node.
        // The child should be a directory.
        // The child should not be "." or "..".
        if name == "." || name == ".." {
            return_errno!(Errno::EPERM);
        }
        if self.lookup(name)?.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        self.remove(name)?;
        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let inode: Arc<dyn Inode> = match name {
            "." => self.this(),
            ".." => self.parent().unwrap_or(self.this()),
            name => self.elem.read().lookup(name)?,
        };
        Ok(inode)
    }

    fn rename(&self, _old_name: &str, _target: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "rename is not supported in pseudo filesystem");
    }

    fn read_link(&self) -> Result<String> {
        if self.type_() != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        self.elem.read().read_link()
    }

    fn write_link(&self, target: &str) -> Result<()> {
        if self.type_() != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        self.elem.write().write_link(target)
    }
}
