// SPDX-License-Identifier: MPL-2.0

//! VFS inode adapter for securityfs SysTree nodes.
//!
//! [`SecurityFsInode`] delegates file I/O directly to [`SecurityDirectory`]
//! so callback errno values are preserved instead of being collapsed by the
//! generic SysTree attribute-error mapping.

use super::{fs::SecurityFs, node::SecurityDirectory};
use crate::{
    fs::{
        file::{InodeMode, InodeType, StatusFlags},
        utils::systree_inode::{SysTreeInodeTy, SysTreeNodeKind},
        vfs::{
            file_system::FileSystem,
            inode::{Extension, FileOps, Inode, Metadata, RevalidationPolicy},
            path::Dentry,
        },
    },
    prelude::*,
};

pub(super) struct SecurityFsInode {
    node_kind: SysTreeNodeKind,
    metadata: Metadata,
    extension: Extension,
    mode: RwLock<InodeMode>,
    parent: Weak<SecurityFsInode>,
    this: Weak<SecurityFsInode>,
}

impl SysTreeInodeTy for SecurityFsInode {
    fn new_arc(
        node_kind: SysTreeNodeKind,
        metadata: Metadata,
        mode: InodeMode,
        parent: Weak<Self>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            node_kind,
            metadata,
            extension: Extension::new(),
            mode: RwLock::new(mode),
            parent,
            this: this.clone(),
        })
    }

    fn node_kind(&self) -> &SysTreeNodeKind {
        &self.node_kind
    }

    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(*self.mode.read())
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        *self.mode.write() = mode;
        Ok(())
    }

    fn parent(&self) -> &Weak<Self> {
        &self.parent
    }

    fn this(&self) -> Arc<Self> {
        self.this
            .upgrade()
            .expect("invalid weak reference to `self`")
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }
}

impl FileOps for SecurityFsInode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let SysTreeNodeKind::Attr(attr, node) = self.node_kind() else {
            return Err(Error::new(Errno::EINVAL));
        };
        let directory = node
            .as_any()
            .downcast_ref::<SecurityDirectory>()
            .ok_or_else(|| Error::new(Errno::EIO))?;
        directory.read_file_at(attr.name(), offset, writer)
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let SysTreeNodeKind::Attr(attr, node) = self.node_kind() else {
            return Err(Error::new(Errno::EINVAL));
        };
        let directory = node
            .as_any()
            .downcast_ref::<SecurityDirectory>()
            .ok_or_else(|| Error::new(Errno::EIO))?;
        directory.write_file_at(attr.name(), offset, reader)
    }
}

impl Inode for SecurityFsInode {
    fn fs(&self) -> Arc<dyn FileSystem> {
        SecurityFs::singleton().clone()
    }

    fn create(
        &self,
        _self_dentry: &Dentry,
        _name: &str,
        _type_: InodeType,
        _mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::with_message(
            Errno::EPERM,
            "file creation under securityfs is not allowed",
        ))
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        match self.node_kind() {
            SysTreeNodeKind::Branch(_) | SysTreeNodeKind::Leaf(_) => {
                RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT
            }
            SysTreeNodeKind::Attr(..) | SysTreeNodeKind::Symlink(_) => RevalidationPolicy::empty(),
        }
    }

    fn revalidate_exists(&self, name: &str, child: &dyn Inode) -> bool {
        let child = child.downcast_ref::<Self>().unwrap();
        let child_node_id = match child.node_kind() {
            SysTreeNodeKind::Attr(attr, node) => {
                return node.node_attrs().get(attr.name()).is_some_and(|current| {
                    current.id() == attr.id() && current.perms() == attr.perms()
                }) && !node.is_attr_absent(name);
            }
            SysTreeNodeKind::Branch(node) => *node.id(),
            SysTreeNodeKind::Leaf(node) => *node.id(),
            SysTreeNodeKind::Symlink(node) => *node.id(),
        };

        match self.node_kind() {
            SysTreeNodeKind::Branch(branch) => branch
                .child(name)
                .is_some_and(|current| *current.id() == child_node_id),
            _ => false,
        }
    }

    fn revalidate_absent(&self, _name: &str) -> bool {
        false
    }
}
