// SPDX-License-Identifier: MPL-2.0

use alloc::{
    borrow::Cow,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::time::Duration;

use aster_systree::{
    SysAttr, SysAttrFlags, SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj, SysStr,
    SysSymlink, SysTree,
};
use ostd::sync::RwLock;

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
    return_errno_with_message,
    time::{clocks::RealTimeCoarseClock, Clock},
    Errno, Error, Result,
};

type Ino = u64;

pub struct SysFsInode {
    /// The global SysTree reference, representing the kernel's exported system information tree.
    systree: &'static Arc<SysTree>,
    /// The corresponding node in the SysTree.
    inner_node: InnerNode,
    /// The metadata of this inode.
    ///
    /// Most of the metadata (e.g., file size, timestamps)
    /// can be determined upon the creation of an inode,
    /// and are thus kept intact inside the immutable `metadata` field.
    /// Currently, the only mutable metadata is `mode`,
    /// which allows user space to `chmod` an inode on sysfs.
    metadata: Metadata,
    /// The file mode (permissions) of this inode, protected by a lock.
    mode: RwLock<InodeMode>,
    /// Weak reference to the parent inode.
    parent: Weak<SysFsInode>,
    /// Weak self-reference for cyclic data structures.
    this: Weak<SysFsInode>,
}

#[derive(Debug)]
enum InnerNode {
    Branch(Arc<dyn SysBranchNode>),
    Leaf(Arc<dyn SysNode>),
    Attr(SysAttr, Arc<dyn SysNode>),
    Symlink(Arc<dyn SysSymlink>),
}

impl SysFsInode {
    pub(crate) fn new_root(systree: &'static Arc<SysTree>) -> Arc<Self> {
        let root_node = systree.root().clone(); // systree.root() returns Arc<dyn SysBranchNode>
        let inner_node = InnerNode::Branch(root_node);
        let parent = Weak::new();
        Self::new_branch_dir(systree, inner_node, parent)
    }

    fn new_attr(
        systree: &'static Arc<SysTree>,
        attr: SysAttr,
        node: Arc<dyn SysNode>,
        parent: Weak<SysFsInode>,
    ) -> Arc<Self> {
        let inner_node = InnerNode::Attr(attr.clone(), node);
        let ino = ino::from_inner_node(&inner_node);
        let metadata = Self::new_metadata(ino, InodeType::File);
        let mode = RwLock::new(Self::flags_to_inode_mode(attr.flags()));
        Arc::new_cyclic(|this| Self {
            systree,
            inner_node,
            metadata,
            mode,
            parent,
            this: this.clone(),
        })
    }

    fn new_symlink(
        systree: &'static Arc<SysTree>,
        symlink: Arc<dyn SysSymlink>,
        parent: Weak<SysFsInode>,
    ) -> Arc<Self> {
        let inner_node = InnerNode::Symlink(symlink);
        let ino = ino::from_inner_node(&inner_node);
        let metadata = Self::new_metadata(ino, InodeType::SymLink);
        let mode = RwLock::new(InodeMode::from_bits_truncate(0o777));
        Arc::new_cyclic(|this| Self {
            systree,
            inner_node,
            metadata,
            mode,
            parent,
            this: this.clone(),
        })
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
            mode: InodeMode::from_bits_truncate(0o555),
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }

    fn flags_to_inode_mode(flags: SysAttrFlags) -> InodeMode {
        let mut bits = 0o000; // Start with no permissions
        if flags.contains(SysAttrFlags::CAN_READ) {
            bits |= 0o444; // Add read permissions if flag is set
        }
        if flags.contains(SysAttrFlags::CAN_WRITE) {
            bits |= 0o222; // Add write permissions if flag is set
        }
        // Note: Execute permissions (0o111) are typically granted for directories, handled elsewhere.
        InodeMode::from_bits_truncate(bits)
    }

    fn new_branch_dir(
        systree: &'static Arc<SysTree>,
        inner_node: InnerNode, // Must be InnerNode::Branch
        parent: Weak<SysFsInode>,
    ) -> Arc<Self> {
        let ino = ino::from_inner_node(&inner_node);
        let metadata = Self::new_metadata(ino, InodeType::Dir);
        let mode = RwLock::new(InodeMode::from_bits_truncate(0o555));
        Arc::new_cyclic(|this| Self {
            systree,
            inner_node,
            metadata,
            mode,
            parent,
            this: this.clone(),
        })
    }

    fn new_leaf_dir(
        systree: &'static Arc<SysTree>,
        inner_node: InnerNode, // Must be InnerNode::Leaf
        parent: Weak<SysFsInode>,
    ) -> Arc<Self> {
        let ino = ino::from_inner_node(&inner_node);
        let metadata = Self::new_metadata(ino, InodeType::Dir); // Leaf nodes are represented as Dirs
        let mode = RwLock::new(InodeMode::from_bits_truncate(0o555)); // Read/execute for all
        Arc::new_cyclic(|this| Self {
            systree,
            inner_node,
            metadata,
            mode,
            parent,
            this: this.clone(),
        })
    }

    pub fn this(&self) -> Arc<SysFsInode> {
        self.this.upgrade().expect("Weak ref invalid")
    }

    fn lookup_node_or_attr(
        &self,
        name: &str,
        sysnode: &dyn SysBranchNode,
    ) -> Result<Arc<dyn Inode>> {
        // Try finding a child node (Branch, Leaf, Symlink) first
        if let Some(child_sysnode) = sysnode.child(name) {
            let child_type = child_sysnode.type_();
            match child_type {
                SysNodeType::Branch => {
                    let child_branch = child_sysnode
                        .arc_as_branch()
                        .ok_or(Error::new(Errno::EIO))?;
                    let inode = Self::new_branch_dir(
                        self.systree,
                        InnerNode::Branch(child_branch),
                        Arc::downgrade(&self.this()),
                    );
                    return Ok(inode);
                }
                SysNodeType::Leaf => {
                    let child_leaf_node =
                        child_sysnode.arc_as_node().ok_or(Error::new(Errno::EIO))?;
                    let inode = Self::new_leaf_dir(
                        self.systree,
                        InnerNode::Leaf(child_leaf_node),
                        Arc::downgrade(&self.this()),
                    );
                    return Ok(inode);
                }
                SysNodeType::Symlink => {
                    let child_symlink = child_sysnode
                        .arc_as_symlink()
                        .ok_or(Error::new(Errno::EIO))?;
                    let inode = Self::new_symlink(
                        self.systree,
                        child_symlink,
                        Arc::downgrade(&self.this()),
                    );
                    return Ok(inode);
                }
            }
        } else {
            // If no child node found, try finding an attribute of the current branch node
            let Some(attr) = sysnode.node_attrs().get(name) else {
                return_errno_with_message!(Errno::ENOENT, "child node or attribute not found");
            };

            let parent_node_arc: Arc<dyn SysNode> = match &self.inner_node {
                InnerNode::Branch(branch_arc) => branch_arc.clone(),
                // This case shouldn't happen if lookup_node_or_attr is called correctly
                _ => {
                    return Err(Error::with_message(
                        Errno::EIO,
                        "lookup_node_or_attr called on non-branch inode",
                    ))
                }
            };

            let inode = Self::new_attr(
                self.systree,
                attr.clone(),
                parent_node_arc,
                Arc::downgrade(&self.this()),
            );
            return Ok(inode);
        }
    }

    fn lookup_attr(&self, name: &str, sysnode: &dyn SysNode) -> Result<Arc<dyn Inode>> {
        // This function is called when the current inode is a Leaf directory
        let Some(attr) = sysnode.node_attrs().get(name) else {
            return Err(Error::new(Errno::ENOENT));
        };

        let leaf_node_arc: Arc<dyn SysNode> = match &self.inner_node {
            InnerNode::Leaf(leaf_arc) => leaf_arc.clone(),
            // This case shouldn't happen if lookup_attr is called correctly
            _ => {
                return Err(Error::with_message(
                    Errno::EIO,
                    "lookup_attr called on non-leaf inode",
                ))
            }
        };

        let inode = Self::new_attr(
            self.systree,
            attr.clone(),
            leaf_node_arc,
            Arc::downgrade(&self.this()),
        );
        Ok(inode)
    }

    fn new_dentry_iter(&self, min_ino: Ino) -> impl Iterator<Item = Dentry> + '_ {
        match &self.inner_node {
            InnerNode::Branch(branch_node) => {
                let attrs = branch_node.node_attrs().iter().cloned().collect();
                let attr_iter = AttrDentryIter::new(attrs, self.ino(), min_ino);
                let child_objs = branch_node.children();
                let node_iter = NodeDentryIter::new(child_objs, min_ino);
                let special_iter = ThisAndParentDentryIter::new(self, min_ino);
                attr_iter.chain(node_iter).chain(special_iter)
            }
            InnerNode::Leaf(leaf_node) => {
                let attrs = leaf_node.node_attrs().iter().cloned().collect();
                let attr_iter = AttrDentryIter::new(attrs, self.ino(), min_ino);
                let node_iter = NodeDentryIter::new(Vec::new(), min_ino);
                let special_iter = ThisAndParentDentryIter::new(self, min_ino);
                attr_iter.chain(node_iter).chain(special_iter)
            }
            InnerNode::Attr(_, _) | InnerNode::Symlink(_) => {
                let attr_iter = AttrDentryIter::new(Vec::new(), self.ino(), min_ino);
                let node_iter = NodeDentryIter::new(Vec::new(), min_ino);
                let special_iter = ThisAndParentDentryIter::new(self, min_ino);
                attr_iter.chain(node_iter).chain(special_iter)
            }
        }
    }
}

impl Inode for SysFsInode {
    fn type_(&self) -> InodeType {
        self.metadata.type_
    }

    fn metadata(&self) -> Metadata {
        self.metadata
    }

    fn ino(&self) -> u64 {
        self.metadata.ino
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(*self.mode.read())
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        *self.mode.write() = mode;
        Ok(())
    }

    fn size(&self) -> usize {
        self.metadata.size
    }

    fn resize(&self, _new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn atime(&self) -> Duration {
        self.metadata.atime
    }
    fn set_atime(&self, _time: Duration) {}

    fn mtime(&self) -> Duration {
        self.metadata.mtime
    }
    fn set_mtime(&self, _time: Duration) {}

    fn ctime(&self) -> Duration {
        self.metadata.ctime
    }
    fn set_ctime(&self, _time: Duration) {}

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.uid)
    }
    fn set_owner(&self, _uid: Uid) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata.gid)
    }
    fn set_group(&self, _gid: Gid) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        crate::fs::sysfs::singleton().clone()
    }

    fn page_cache(&self) -> Option<crate::vm::vmo::Vmo<aster_rights::Full>> {
        None
    }

    fn read_at(&self, offset: usize, buf: &mut VmWriter) -> Result<usize> {
        self.read_direct_at(offset, buf)
    }

    fn read_direct_at(&self, _offset: usize, buf: &mut VmWriter) -> Result<usize> {
        // TODO: it is unclear whether we should simply ignore the offset
        // or report errors if it is non-zero.

        let InnerNode::Attr(attr, leaf) = &self.inner_node else {
            return Err(Error::new(Errno::EINVAL));
        };

        // TODO: check read permission
        Ok(leaf.read_attr(attr.name(), buf)?)
    }

    fn write_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        self.write_direct_at(offset, buf)
    }

    fn write_direct_at(&self, _offset: usize, buf: &mut VmReader) -> Result<usize> {
        let InnerNode::Attr(attr, leaf) = &self.inner_node else {
            return Err(Error::new(Errno::EINVAL));
        };

        // TODO: check write permission
        Ok(leaf.write_attr(attr.name(), buf)?)
    }

    fn create(&self, _name: &str, _type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    fn mknod(&self, _name: &str, _mode: InodeMode, _dev: MknodType) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn unlink(&self, _name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn rename(&self, _old_name: &str, _target: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if self.type_() != InodeType::Dir {
            return Err(Error::new(Errno::ENOTDIR));
        }

        if name == "." {
            return Ok(self.this());
        } else if name == ".." {
            return Ok(self.parent.upgrade().unwrap_or_else(|| self.this()));
        }

        // Dispatch based on the concrete type of the directory inode
        match &self.inner_node {
            InnerNode::Branch(branch_node) => {
                // Current inode is a directory corresponding to a SysBranchNode
                self.lookup_node_or_attr(name, branch_node.as_ref())
            }
            InnerNode::Leaf(leaf_node) => {
                // Current inode is a directory corresponding to a SysNode (Leaf)
                // Leaf directories only contain attributes, not other nodes.
                self.lookup_attr(name, leaf_node.as_ref())
            }
            // Attr and Symlink nodes are not directories
            InnerNode::Attr(_, _) | InnerNode::Symlink(_) => Err(Error::new(Errno::ENOTDIR)),
        }
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
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

        let mut iter = self.new_dentry_iter(start_ino + 1);

        while let Some(dentry) = iter.next() {
            // The offset reported back to the caller should be the absolute position
            let next_offset = (dentry.ino + 1) as usize;
            let res = visitor.visit(&dentry.name, dentry.ino, dentry.type_, next_offset);

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
            Ok(0)
        } else {
            // Return absolute offset instead of an increment
            Ok((last_ino + 1) as usize)
        }
    }

    fn read_link(&self) -> Result<String> {
        match &self.inner_node {
            InnerNode::Symlink(s) => Ok(s.target_path().to_string()),
            _ => Err(Error::new(Errno::EINVAL)),
        }
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn as_device(&self) -> Option<Arc<dyn Device>> {
        None
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        Err(Error::new(Errno::ENOTTY))
    }

    fn sync_all(&self) -> Result<()> {
        Ok(())
    }

    fn sync_data(&self) -> Result<()> {
        Ok(())
    }

    fn fallocate(&self, _mode: FallocMode, _offset: usize, _len: usize) -> Result<()> {
        Err(Error::new(Errno::EOPNOTSUPP))
    }

    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let mut events = IoEvents::empty();
        if let InnerNode::Attr(attr, _) = &self.inner_node {
            if attr.flags().contains(SysAttrFlags::CAN_READ) {
                events |= IoEvents::IN;
            }
            if attr.flags().contains(SysAttrFlags::CAN_WRITE) {
                events |= IoEvents::OUT;
            }
        }
        events & mask
    }

    fn is_dentry_cacheable(&self) -> bool {
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
                    name: obj.name(),
                    type_,
                });
            }
        }
        None
    }
}

struct ThisAndParentDentryIter<'a> {
    inode: &'a SysFsInode,
    min_ino: Ino,
    state: u8, // 0 = self, 1 = parent, 2 = done
}

impl<'a> ThisAndParentDentryIter<'a> {
    fn new(inode: &'a SysFsInode, min_ino: Ino) -> Self {
        Self {
            inode,
            min_ino,
            state: 0,
        }
    }
}

impl<'a> Iterator for ThisAndParentDentryIter<'a> {
    type Item = Dentry;

    fn next(&mut self) -> Option<Dentry> {
        match self.state {
            0 => {
                self.state = 1;
                if self.inode.ino() >= self.min_ino {
                    Some(Dentry {
                        ino: self.inode.ino(),
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
                    .parent
                    .upgrade()
                    .map_or(self.inode.ino(), |p| p.ino());
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
    use super::{InnerNode, Ino, SysNodeId};

    const ATTR_ID_BITS: u8 = 8;

    pub fn from_sysnode_id(node_id: &SysNodeId) -> Ino {
        node_id.as_u64() << ATTR_ID_BITS
    }

    pub fn from_dir_ino_and_attr_id(dir_ino: Ino, attr_id: u8) -> Ino {
        dir_ino + (attr_id as Ino)
    }

    pub fn from_inner_node(inner: &InnerNode) -> Ino {
        match inner {
            InnerNode::Branch(branch_node) => from_sysnode_id(branch_node.id()),
            InnerNode::Leaf(leaf_node) => from_sysnode_id(leaf_node.id()),
            InnerNode::Symlink(symlink_node) => from_sysnode_id(symlink_node.id()),
            InnerNode::Attr(attr, node) => {
                // node here is the parent (Branch or Leaf)
                let dir_ino = from_sysnode_id(node.id()); // Get parent dir ino
                from_dir_ino_and_attr_id(dir_ino, attr.id())
            }
        }
    }
}
