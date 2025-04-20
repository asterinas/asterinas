// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

extern crate alloc;

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::time::Duration;

use spin::RwLock;
use systree::{
    SysAttr, SysAttrFlags, SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj, SysSymlink,
    SysTree,
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
    Branch(Arc<dyn SysObj>),
    Attr(SysAttr, Arc<dyn SysNode>),
    Symlink(Arc<dyn SysSymlink>),
}

/// A directory entry of sysfs.
struct Dentry {
    pub ino: Ino,
    pub name: alloc::string::String,
    pub type_: InodeType,
}

impl SysFsInode {
    fn new_dentry_iter(&self, min_ino: Ino) -> Box<dyn Iterator<Item = Dentry> + '_> {
        match &self.inner_node {
            InnerNode::Branch(obj) => {
                if let Some(branch) = obj.as_branch() {
                    let attr_iter = branch.node_attrs().iter().filter_map(move |attr| {
                        let ino = ino::from_dir_ino_and_attr_id(self.ino(), attr.id());
                        if ino < min_ino {
                            None
                        } else {
                            Some(Dentry {
                                ino,
                                name: attr.name().to_string(),
                                type_: InodeType::File,
                            })
                        }
                    });

                    let node_iter = branch.children().into_iter().filter_map(move |child| {
                        let ino = ino::from_sysnode_id(child.id());
                        if ino < min_ino {
                            None
                        } else {
                            let type_ = match child.type_() {
                                SysNodeType::Branch | SysNodeType::Leaf => InodeType::Dir,
                                SysNodeType::Symlink => InodeType::SymLink,
                            };
                            Some(Dentry {
                                ino,
                                name: child.name().to_string(),
                                type_,
                            })
                        }
                    });

                    let this_and_parent_iter = Self::this_and_parent_dentry_iter(self, min_ino);

                    Box::new(attr_iter.chain(node_iter).chain(this_and_parent_iter))
                } else if let Some(leaf) = obj.as_node() {
                    let attr_iter = leaf.node_attrs().iter().filter_map(move |attr| {
                        let ino = ino::from_dir_ino_and_attr_id(self.ino(), attr.id());
                        if ino < min_ino {
                            None
                        } else {
                            Some(Dentry {
                                ino,
                                name: attr.name().to_string(),
                                type_: InodeType::File,
                            })
                        }
                    });

                    let node_iter = core::iter::empty();

                    let this_and_parent_iter = Self::this_and_parent_dentry_iter(self, min_ino);

                    Box::new(attr_iter.chain(node_iter).chain(this_and_parent_iter))
                } else {
                    panic!("new_dentry_iter called on non-dir inode");
                }
            }
            _ => panic!("new_dentry_iter called on non-dir inode"),
        }
    }

    fn this_and_parent_dentry_iter<'a>(
        inode: &'a SysFsInode,
        min_ino: Ino,
    ) -> impl Iterator<Item = Dentry> + 'a {
        let self_ino = inode.ino();
        let parent_ino = inode.parent.upgrade().map_or(self_ino, |p| p.ino());

        let mut entries = Vec::new();

        if self_ino >= min_ino {
            entries.push(Dentry {
                ino: self_ino,
                name: ".".to_string(),
                type_: InodeType::Dir,
            });
        }

        if parent_ino >= min_ino {
            entries.push(Dentry {
                ino: parent_ino,
                name: "..".to_string(),
                type_: InodeType::Dir,
            });
        }

        entries.into_iter()
    }
    pub(crate) fn new_root(systree: &'static Arc<SysTree>) -> Arc<Self> {
        let root_node = systree.root().clone();
        let inner_node = InnerNode::Branch(root_node);
        let parent = Weak::new();
        Self::new_dir(systree, inner_node, parent)
    }

    fn new_dir(
        systree: &'static Arc<SysTree>,
        inner_node: InnerNode,
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
        let inner_node = InnerNode::Symlink(symlink.clone());
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
        let mut bits = 0o444;
        if flags.contains(SysAttrFlags::CAN_WRITE) {
            bits |= 0o222;
        }
        InodeMode::from_bits_truncate(bits)
    }

    pub fn this(&self) -> Arc<SysFsInode> {
        self.this.upgrade().expect("Weak ref invalid")
    }

    fn lookup_node_or_attr(
        &self,
        name: &str,
        sysnode: &dyn SysBranchNode,
    ) -> Result<Arc<dyn Inode>> {
        if let Some(child_sysnode) = sysnode.child(name) {
            let child_type = child_sysnode.type_();
            match child_type {
                SysNodeType::Branch => {
                    let inode = Self::new_dir(
                        self.systree,
                        InnerNode::Branch(child_sysnode.clone()),
                        Arc::downgrade(&self.this()),
                    );
                    return Ok(inode);
                }
                SysNodeType::Leaf => {
                    let Some(attr) = sysnode.node_attrs().get(name) else {
                        return_errno_with_message!(Errno::ENOENT, "attr not found");
                    };
                    let inode = Self::new_attr(
                        self.systree,
                        attr.clone(),
                        sysnode.arc_as_node().expect("not a SysNode"),
                        Arc::downgrade(&self.this()),
                    );
                    return Ok(inode);
                }
                SysNodeType::Symlink => {
                    let inode = Self::new_symlink(
                        self.systree,
                        child_sysnode.arc_as_symlink().expect("not a symlink"),
                        Arc::downgrade(&self.this()),
                    );
                    return Ok(inode);
                }
            }
        } else {
            return_errno_with_message!(Errno::ENOENT, "child not found");
        }
    }

    fn lookup_attr(&self, name: &str, sysnode: &dyn SysNode) -> Result<Arc<dyn Inode>> {
        let Some(attr) = sysnode.node_attrs().get(name) else {
            return Err(Error::new(Errno::ENOENT));
        };
        let inode = Self::new_attr(
            self.systree,
            attr.clone(),
            sysnode.arc_as_node().expect("not a SysNode"),
            Arc::downgrade(&self.this()),
        );
        Ok(inode)
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

        Err(Error::new(Errno::EINVAL))
    }

    fn write_at(&self, offset: usize, buf: &mut VmReader) -> Result<usize> {
        self.write_direct_at(offset, buf)
    }

    fn write_direct_at(&self, _offset: usize, buf: &mut VmReader) -> Result<usize> {
        let InnerNode::Attr(attr, leaf) = &self.inner_node else {
            return Err(Error::new(Errno::EINVAL));
        };

        // TODO: check write permission

        Err(Error::new(Errno::EINVAL))
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
        match &self.inner_node {
            InnerNode::Branch(obj) => {
                if let Some(branch) = obj.as_branch() {
                    self.lookup_node_or_attr(name, branch)
                } else if let Some(node) = obj.as_node() {
                    self.lookup_attr(name, node)
                } else {
                    Err(Error::new(Errno::ENOTDIR))
                }
            }
            _ => Err(Error::new(Errno::ENOTDIR)),
        }
    }

    /// Reads directory entries starting from a given offset.
    ///
    /// Why interpreting the `offset` argument as an inode number?
    ///
    /// It may take multiple `getdents` system calls
    /// -- and thus multiple calls to this method --
    /// to list a large directory when the syscall is provided a small buffer.
    /// Between these calls,
    /// the directory may have new entries added or existing ones removed
    /// by some concurrent users that are working on the directory.
    /// In such situations,
    /// missing some of the concurrently-added entries is inevitable,
    /// but reporting the same entry multiple times would be
    /// very confusing to the user.
    ///
    /// To address this issue,
    /// the `readdir_at` method reports entries starting from a user-given `offset`
    /// and returns an increment that the next call should be put on the `offset` argument
    /// to avoid getting duplicated entries.
    ///
    /// Different file systems may interpret the meaning of
    /// the `offset` argument differently:
    /// one may take it as a _byte_ offset,
    /// while the other may treat it as an _index_.
    /// This freedom is guaranteed by Linux as documented in
    /// [the man page of getdents](https://man7.org/linux/man-pages/man2/getdents.2.html).
    ///
    /// Our implementation of sysfs interprets the `offset`
    /// as an _inode number_.
    /// By inode numbers, directory entries will have a _stable_ order
    /// across different calls to `readdir_at`.
    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let start_ino = offset as Ino;
        let mut count = 0;
        let mut last_ino = start_ino;

        // Collect all entries
        let mut entries: Vec<_> = self.new_dentry_iter(0).collect();

        // Sort by ino then name for deterministic order
        entries.sort_by(|a, b| a.ino.cmp(&b.ino).then_with(|| a.name.cmp(&b.name)));

        // Deduplicate by ino, keeping first occurrence
        entries.dedup_by_key(|d| d.ino);

        // Skip entries with ino < start_ino
        let mut iter = entries
            .into_iter()
            .skip_while(|d| d.ino < start_ino)
            .peekable();

        while let Some(dentry) = iter.next() {
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
            Ok((last_ino + 1 - start_ino) as usize)
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
            InnerNode::Branch(obj) => {
                if let Some(branch) = obj.as_branch() {
                    from_sysnode_id(branch.id())
                } else if let Some(node) = obj.as_node() {
                    from_sysnode_id(node.id())
                } else {
                    0
                }
            }
            InnerNode::Symlink(node) => from_sysnode_id(node.id()),
            InnerNode::Attr(attr, node) => {
                let dir_ino = from_sysnode_id(node.id());
                from_dir_ino_and_attr_id(dir_ino, attr.id())
            }
        }
    }
}
