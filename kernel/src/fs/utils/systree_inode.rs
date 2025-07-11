// SPDX-License-Identifier: MPL-2.0

use alloc::{
    borrow::Cow,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::time::Duration;

use aster_systree::{
    SysAttr, SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj, SysStr, SysSymlink,
};

use crate::{
    events::IoEvents,
    fs::{
        device::Device,
        utils::{
            DirentVisitor, FallocMode, FileSystem, Inode, InodeMode, InodeType, IoctlCmd, Metadata,
            MknodType,
        },
    },
    prelude::{VmReader, VmWriter},
    process::{signal::PollHandle, Gid, Uid},
    return_errno, return_errno_with_message,
    time::{clocks::RealTimeCoarseClock, Clock},
    Errno, Error, Result,
};

type Ino = u64;

/// A trait that abstracts an inode type backed by a `SysTree` node,
/// e.g., a `SysFs` inode and a `CgroupFs` inode.
///
/// The struct implementing this trait will have a default implementation for
/// the [`Inode`] trait. Users only need to additionally implement the
/// [`Inode::fs`] method.
#[expect(dead_code)]
pub(in crate::fs) trait SysTreeInodeTy: Send + Sync + 'static {
    fn new_arc(
        node_kind: SysTreeNodeKind,
        metadata: Metadata,
        mode: InodeMode,
        parent: Weak<Self>,
    ) -> Arc<Self>
    where
        Self: Sized + 'static;

    fn node_kind(&self) -> &SysTreeNodeKind;

    fn metadata(&self) -> &Metadata;

    fn mode(&self) -> Result<InodeMode>;

    fn set_mode(&self, mode: InodeMode) -> Result<()>;

    fn parent(&self) -> &Weak<Self>;

    fn this(&self) -> Arc<Self>
    where
        Self: Sized + 'static;

    fn new_root(root_node: Arc<dyn SysBranchNode>) -> Arc<Self>
    where
        Self: Sized + 'static,
    {
        let node_kind = SysTreeNodeKind::Branch(root_node);
        let parent = Weak::new();
        Self::new_branch_dir(node_kind, None, parent)
    }

    fn new_metadata(ino: u64, type_: InodeType) -> Metadata {
        let now = RealTimeCoarseClock::get().read_time();
        Metadata {
            dev: 0,
            ino,
            size: 0,
            blk_size: 1024,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_,
            // The mode field in metadata will not be used
            mode: InodeMode::from_bits_truncate(0o000),
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }

    fn new_attr(attr: SysAttr, node: Arc<dyn SysNode>, parent: Weak<Self>) -> Arc<Self>
    where
        Self: Sized + 'static,
    {
        let node_kind = SysTreeNodeKind::Attr(attr.clone(), node);
        let ino = ino::from_node_kind(&node_kind);
        let metadata = Self::new_metadata(ino, InodeType::File);
        let mode = attr.perms().into();
        Self::new_arc(node_kind, metadata, mode, parent)
    }

    fn new_symlink(symlink: Arc<dyn SysSymlink>, parent: Weak<Self>) -> Arc<Self>
    where
        Self: Sized + 'static,
    {
        let node_kind = SysTreeNodeKind::Symlink(symlink);
        let ino = ino::from_node_kind(&node_kind);
        let metadata = Self::new_metadata(ino, InodeType::SymLink);
        let mode = InodeMode::from_bits_truncate(0o777);
        Self::new_arc(node_kind, metadata, mode, parent)
    }

    fn new_branch_dir(
        node_kind: SysTreeNodeKind, // Must be SysTreeNodeKind::Branch
        mode: Option<InodeMode>,
        parent: Weak<Self>,
    ) -> Arc<Self>
    where
        Self: Sized + 'static,
    {
        let ino = ino::from_node_kind(&node_kind);
        let metadata = Self::new_metadata(ino, InodeType::Dir);
        let SysTreeNodeKind::Branch(branch_node) = &node_kind else {
            panic!("new_branch_dir called with non-branch SysTreeNodeKind");
        };

        let mode = mode.unwrap_or_else(|| branch_node.perms().into());
        Self::new_arc(node_kind, metadata, mode, parent)
    }

    fn new_leaf_dir(
        node_kind: SysTreeNodeKind, // Must be SysTreeNodeKind::Leaf
        mode: Option<InodeMode>,
        parent: Weak<Self>,
    ) -> Arc<Self>
    where
        Self: Sized + 'static,
    {
        let ino = ino::from_node_kind(&node_kind);
        let metadata = Self::new_metadata(ino, InodeType::Dir); // Leaf nodes are represented as Dirs

        let SysTreeNodeKind::Leaf(leaf_node) = &node_kind else {
            panic!("new_leaf_dir called with non-leaf SysTreeNodeKind");
        };

        let mode = mode.unwrap_or_else(|| leaf_node.perms().into());
        Self::new_arc(node_kind, metadata, mode, parent)
    }

    fn lookup_node_or_attr(&self, name: &str, sysnode: &dyn SysBranchNode) -> Result<Arc<dyn Inode>>
    where
        Self: Sized + 'static,
    {
        // Try finding a child node (Branch, Leaf, Symlink) first
        if let Some(child_sysnode) = sysnode.child(name) {
            let child_type = child_sysnode.type_();
            match child_type {
                SysNodeType::Branch => {
                    let child_branch = child_sysnode
                        .cast_to_branch()
                        .ok_or(Error::new(Errno::EIO))?;
                    let inode = Self::new_branch_dir(
                        SysTreeNodeKind::Branch(child_branch),
                        None,
                        Arc::downgrade(&self.this()),
                    );
                    Ok(inode)
                }
                SysNodeType::Leaf => {
                    let child_leaf_node =
                        child_sysnode.cast_to_node().ok_or(Error::new(Errno::EIO))?;
                    let inode = Self::new_leaf_dir(
                        SysTreeNodeKind::Leaf(child_leaf_node),
                        None,
                        Arc::downgrade(&self.this()),
                    );
                    Ok(inode)
                }
                SysNodeType::Symlink => {
                    let child_symlink = child_sysnode
                        .cast_to_symlink()
                        .ok_or(Error::new(Errno::EIO))?;
                    let inode = Self::new_symlink(child_symlink, Arc::downgrade(&self.this()));
                    Ok(inode)
                }
            }
        } else {
            // If no child node found, try finding an attribute of the current branch node
            let Some(attr) = sysnode.node_attrs().get(name) else {
                return_errno_with_message!(Errno::ENOENT, "child node or attribute not found");
            };

            let parent_node_arc: Arc<dyn SysNode> = match &self.node_kind() {
                SysTreeNodeKind::Branch(branch_arc) => branch_arc.clone(),
                // This case shouldn't happen if lookup_node_or_attr is called correctly
                _ => {
                    return Err(Error::with_message(
                        Errno::EIO,
                        "lookup_node_or_attr called on non-branch inode",
                    ))
                }
            };

            let inode = Self::new_attr(attr.clone(), parent_node_arc, Arc::downgrade(&self.this()));
            Ok(inode)
        }
    }

    fn lookup_attr(&self, name: &str, sysnode: &dyn SysNode) -> Result<Arc<dyn Inode>>
    where
        Self: Sized + 'static,
    {
        // This function is called when the current inode is a Leaf directory
        let Some(attr) = sysnode.node_attrs().get(name) else {
            return Err(Error::new(Errno::ENOENT));
        };

        let leaf_node_arc: Arc<dyn SysNode> = match &self.node_kind() {
            SysTreeNodeKind::Leaf(leaf_arc) => leaf_arc.clone(),
            // This case shouldn't happen if lookup_attr is called correctly
            _ => {
                return Err(Error::with_message(
                    Errno::EIO,
                    "lookup_attr called on non-leaf inode",
                ))
            }
        };

        let inode = Self::new_attr(attr.clone(), leaf_node_arc, Arc::downgrade(&self.this()));
        Ok(inode)
    }

    fn new_dentry_iter(&self, min_ino: Ino) -> impl Iterator<Item = Dentry> + '_
    where
        Self: Sized + 'static,
    {
        match &self.node_kind() {
            SysTreeNodeKind::Branch(branch_node) => {
                let attrs = branch_node.node_attrs().iter().cloned().collect();
                let attr_iter = AttrDentryIter::new(attrs, self.metadata().ino, min_ino);
                let child_objs = branch_node.children();
                let node_iter = NodeDentryIter::new(child_objs, min_ino);
                let special_iter = ThisAndParentDentryIter::new(self, min_ino);
                attr_iter.chain(node_iter).chain(special_iter)
            }
            SysTreeNodeKind::Leaf(leaf_node) => {
                let attrs = leaf_node.node_attrs().iter().cloned().collect();
                let attr_iter = AttrDentryIter::new(attrs, self.metadata().ino, min_ino);
                let node_iter = NodeDentryIter::new(Vec::new(), min_ino);
                let special_iter = ThisAndParentDentryIter::new(self, min_ino);
                attr_iter.chain(node_iter).chain(special_iter)
            }
            SysTreeNodeKind::Attr(_, _) | SysTreeNodeKind::Symlink(_) => {
                let attr_iter = AttrDentryIter::new(Vec::new(), self.metadata().ino, min_ino);
                let node_iter = NodeDentryIter::new(Vec::new(), min_ino);
                let special_iter = ThisAndParentDentryIter::new(self, min_ino);
                attr_iter.chain(node_iter).chain(special_iter)
            }
        }
    }

    fn is_dir(&self) -> bool {
        matches!(
            self.node_kind(),
            SysTreeNodeKind::Branch(_) | SysTreeNodeKind::Leaf(_)
        )
    }
}

/// An enum that represent one of the variants of `SysTree` nodes.
#[derive(Debug)]
pub(in crate::fs) enum SysTreeNodeKind {
    Branch(Arc<dyn SysBranchNode>),
    Leaf(Arc<dyn SysNode>),
    Attr(SysAttr, Arc<dyn SysNode>),
    Symlink(Arc<dyn SysSymlink>),
}

impl<KInode: SysTreeInodeTy + Send + Sync + 'static> Inode for KInode {
    default fn type_(&self) -> InodeType {
        self.metadata().type_
    }

    default fn metadata(&self) -> Metadata {
        let mut metadata = *self.metadata();
        metadata.mode = self.mode().unwrap();
        metadata
    }

    default fn ino(&self) -> u64 {
        self.metadata().ino
    }

    default fn mode(&self) -> Result<InodeMode> {
        self.mode()
    }

    default fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.set_mode(mode)
    }

    default fn size(&self) -> usize {
        self.metadata().size
    }

    default fn resize(&self, _new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    default fn atime(&self) -> Duration {
        self.metadata().atime
    }

    default fn set_atime(&self, _time: Duration) {}

    default fn mtime(&self) -> Duration {
        self.metadata().mtime
    }

    default fn set_mtime(&self, _time: Duration) {}

    default fn ctime(&self) -> Duration {
        self.metadata().ctime
    }

    default fn set_ctime(&self, _time: Duration) {}

    default fn owner(&self) -> Result<Uid> {
        Ok(self.metadata().uid)
    }

    default fn set_owner(&self, _uid: Uid) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    default fn group(&self) -> Result<Gid> {
        Ok(self.metadata().gid)
    }

    default fn set_group(&self, _gid: Gid) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    default fn fs(&self) -> Arc<dyn FileSystem> {
        unimplemented!("fs() method should be implemented by the concrete inode type");
    }

    default fn page_cache(&self) -> Option<crate::vm::vmo::Vmo<aster_rights::Full>> {
        None
    }

    default fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        self.read_direct_at(offset, buf)
    }

    default fn read_direct_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        let SysTreeNodeKind::Attr(attr, leaf) = &self.node_kind() else {
            return Err(Error::new(Errno::EINVAL));
        };

        let len = if offset == 0 {
            leaf.read_attr(attr.name(), buf)?
        } else {
            // The `read_attr_at` method is more general than `read_attr`,
            // but it could be less efficient. So we only use the more general form when necessary.
            leaf.read_attr_at(attr.name(), offset, buf)?
        };

        Ok(len)
    }

    default fn write_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        self.write_direct_at(offset, buf)
    }

    default fn write_direct_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        let SysTreeNodeKind::Attr(attr, leaf) = &self.node_kind() else {
            return Err(Error::new(Errno::EINVAL));
        };

        let len = if offset == 0 {
            leaf.write_attr(attr.name(), buf)?
        } else {
            leaf.write_attr_at(attr.name(), offset, buf)?
        };

        Ok(len)
    }

    default fn create(
        &self,
        name: &str,
        _type_: InodeType,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        if name.len() > super::NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }

        let SysTreeNodeKind::Branch(branch_node) = &self.node_kind() else {
            return_errno_with_message!(Errno::ENOTDIR, "self is not a dir");
        };

        let new_child = branch_node.create_child(name)?;

        let new_inode = if let Some(branch_child) = new_child.cast_to_branch() {
            Self::new_branch_dir(
                SysTreeNodeKind::Branch(branch_child),
                Some(mode),
                self.parent().clone(),
            )
        } else {
            Self::new_leaf_dir(
                SysTreeNodeKind::Leaf(new_child.cast_to_node().unwrap()),
                Some(mode),
                self.parent().clone(),
            )
        };

        Ok(new_inode)
    }

    default fn mknod(
        &self,
        _name: &str,
        _mode: InodeMode,
        _dev: MknodType,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    default fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    default fn unlink(&self, _name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    default fn rename(
        &self,
        _old_name: &str,
        _target: &Arc<dyn Inode>,
        _new_name: &str,
    ) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    default fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if self.type_() != InodeType::Dir {
            return Err(Error::new(Errno::ENOTDIR));
        }

        if name == "." {
            return Ok(self.this());
        } else if name == ".." {
            return Ok(self.parent().upgrade().unwrap_or_else(|| self.this()));
        }

        // Dispatch based on the concrete type of the directory inode
        match &self.node_kind() {
            SysTreeNodeKind::Branch(branch_node) => {
                // Current inode is a directory corresponding to a SysBranchNode
                self.lookup_node_or_attr(name, branch_node.as_ref())
            }
            SysTreeNodeKind::Leaf(leaf_node) => {
                // Current inode is a directory corresponding to a SysNode (Leaf)
                // Leaf directories only contain attributes, not other nodes.
                self.lookup_attr(name, leaf_node.as_ref())
            }
            // Attr and Symlink nodes are not directories
            SysTreeNodeKind::Attr(_, _) | SysTreeNodeKind::Symlink(_) => {
                Err(Error::new(Errno::ENOTDIR))
            }
        }
    }

    default fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        // Why interpreting the `offset` argument as an inode number?
        //
        // It may take multiple `getdents` system calls
        // -- and thus multiple calls to this method --
        // to list a large directory when the syscall is provided a small buffer.
        // Between these calls,
        // the directory may have new entries added or existing ones removed
        // by some concurrent users that are working on the directory.
        // In such situations,
        // missing some of the concurrently-added entries is inevitable,
        // but reporting the same entry multiple times would be
        // very confusing to the user.
        //
        // To address this issue,
        // the `readdir_at` method reports entries starting from a user-given `offset`
        // and returns an increment that the next call should be put on the `offset` argument
        // to avoid getting duplicated entries.
        //
        // Different file systems may interpret the meaning of
        // the `offset` argument differently:
        // one may take it as a _byte_ offset,
        // while the other may treat it as an _index_.
        // This freedom is guaranteed by Linux as documented in
        // [the man page of getdents](https://man7.org/linux/man-pages/man2/getdents.2.html).
        //
        // Our implementation of sysfs interprets the `offset`
        // as an _inode number_.
        // By inode numbers, directory entries will have a _stable_ order
        // across different calls to `readdir_at`.
        // The `new_dentry_iter` is responsible for filtering out entries
        // with inode numbers less than `start_ino`.
        let start_ino = offset as Ino;
        let mut count = 0;
        let mut last_ino = start_ino;

        let dentries = {
            let mut dentries: Vec<_> = self.new_dentry_iter(start_ino).collect();
            dentries.sort_by_key(|d| d.ino);
            dentries
        };

        for dentry in dentries {
            let res = visitor.visit(&dentry.name, dentry.ino, dentry.type_, dentry.ino as usize);

            if res.is_err() {
                if count == 0 {
                    return Err(Error::new(Errno::EINVAL));
                } else {
                    break;
                }
            }
            count += 1;
            last_ino = dentry.ino;
        }

        if count == 0 {
            return Ok(0);
        }

        let next_ino = last_ino + 1;
        Ok((next_ino - start_ino) as usize)
    }

    default fn read_link(&self) -> Result<String> {
        match &self.node_kind() {
            SysTreeNodeKind::Symlink(s) => Ok(s.target_path().to_string()),
            _ => Err(Error::new(Errno::EINVAL)),
        }
    }

    default fn write_link(&self, _target: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    default fn as_device(&self) -> Option<Arc<dyn Device>> {
        None
    }

    default fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        Err(Error::new(Errno::ENOTTY))
    }

    default fn sync_all(&self) -> Result<()> {
        Ok(())
    }

    default fn sync_data(&self) -> Result<()> {
        Ok(())
    }

    default fn fallocate(&self, _mode: FallocMode, _offset: usize, _len: usize) -> Result<()> {
        Err(Error::new(Errno::EOPNOTSUPP))
    }

    default fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let mut events = IoEvents::empty();
        if let SysTreeNodeKind::Attr(attr, _) = &self.node_kind() {
            if attr.perms().can_read() {
                events |= IoEvents::IN;
            }
            if attr.perms().can_write() {
                events |= IoEvents::OUT;
            }
        }
        events & mask
    }

    default fn is_dentry_cacheable(&self) -> bool {
        true
    }
}

// Update AttrDentryIter to filter by min_ino
struct AttrDentryIter {
    attrs: Vec<SysAttr>,
    dir_ino: Ino,
    min_ino: Ino,
    index: usize,
}

impl AttrDentryIter {
    fn new(attrs: Vec<SysAttr>, dir_ino: Ino, min_ino: Ino) -> Self {
        Self {
            attrs,
            dir_ino,
            min_ino,
            index: 0,
        }
    }
}

impl Iterator for AttrDentryIter {
    type Item = Dentry;

    fn next(&mut self) -> Option<Dentry> {
        while self.index < self.attrs.len() {
            let attr = &self.attrs[self.index];
            self.index += 1;
            let attr_ino = ino::from_dir_ino_and_attr_id(self.dir_ino, attr.id());

            if attr_ino >= self.min_ino {
                // Filter by min_ino
                return Some(Dentry {
                    ino: attr_ino,
                    name: attr.name().clone(),
                    type_: InodeType::File,
                });
            }
        }
        None
    }
}

struct NodeDentryIter {
    nodes: Vec<Arc<dyn SysObj>>,
    min_ino: Ino,
    index: usize,
}

impl NodeDentryIter {
    fn new(nodes: Vec<Arc<dyn SysObj>>, min_ino: Ino) -> Self {
        Self {
            nodes,
            min_ino,
            index: 0,
        }
    }
}

impl Iterator for NodeDentryIter {
    type Item = Dentry;

    fn next(&mut self) -> Option<Dentry> {
        while self.index < self.nodes.len() {
            let obj = &self.nodes[self.index];
            self.index += 1;
            let obj_ino = ino::from_sysnode_id(obj.id());

            if obj_ino >= self.min_ino {
                // Filter by min_ino here
                let type_ = match obj.type_() {
                    SysNodeType::Branch => InodeType::Dir,
                    // Leaf nodes are presented as directories containing their attributes
                    SysNodeType::Leaf => InodeType::Dir,
                    SysNodeType::Symlink => InodeType::SymLink,
                };
                return Some(Dentry {
                    ino: obj_ino,
                    name: obj.name().clone(),
                    type_,
                });
            }
        }
        None
    }
}

struct ThisAndParentDentryIter<'a, KInode: SysTreeInodeTy> {
    inode: &'a KInode,
    min_ino: Ino,
    state: u8, // 0 = self, 1 = parent, 2 = done
}

impl<'a, KInode: SysTreeInodeTy> ThisAndParentDentryIter<'a, KInode> {
    fn new(inode: &'a KInode, min_ino: Ino) -> Self {
        Self {
            inode,
            min_ino,
            state: 0,
        }
    }
}

impl<KInode: SysTreeInodeTy> Iterator for ThisAndParentDentryIter<'_, KInode> {
    type Item = Dentry;

    fn next(&mut self) -> Option<Dentry> {
        match self.state {
            0 => {
                self.state = 1;
                if self.inode.metadata().ino >= self.min_ino {
                    Some(Dentry {
                        ino: self.inode.metadata().ino,
                        name: Cow::from("."),
                        type_: InodeType::Dir,
                    })
                } else {
                    self.next()
                }
            }
            1 => {
                self.state = 2;
                let parent_ino = self
                    .inode
                    .parent()
                    .upgrade()
                    .map_or(self.inode.metadata().ino, |p| p.metadata().ino);
                if parent_ino >= self.min_ino {
                    Some(Dentry {
                        ino: parent_ino,
                        name: Cow::from(".."),
                        type_: InodeType::Dir,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// A directory entry of sysfs.
struct Dentry {
    pub ino: Ino,
    pub name: SysStr,
    pub type_: InodeType,
}

mod ino {
    use super::{Ino, SysNodeId, SysTreeNodeKind};

    const ATTR_ID_BITS: u8 = 8;

    pub fn from_sysnode_id(node_id: &SysNodeId) -> Ino {
        node_id.as_u64() << ATTR_ID_BITS
    }

    pub fn from_dir_ino_and_attr_id(dir_ino: Ino, attr_id: u8) -> Ino {
        dir_ino + (attr_id as Ino)
    }

    pub fn from_node_kind(inner: &SysTreeNodeKind) -> Ino {
        match inner {
            SysTreeNodeKind::Branch(branch_node) => from_sysnode_id(branch_node.id()),
            SysTreeNodeKind::Leaf(leaf_node) => from_sysnode_id(leaf_node.id()),
            SysTreeNodeKind::Symlink(symlink_node) => from_sysnode_id(symlink_node.id()),
            SysTreeNodeKind::Attr(attr, node) => {
                // node here is the parent (Branch or Leaf)
                let dir_ino = from_sysnode_id(node.id()); // Get parent dir ino
                from_dir_ino_and_attr_id(dir_ino, attr.id())
            }
        }
    }
}
