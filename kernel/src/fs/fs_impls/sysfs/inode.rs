// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use ostd::sync::RwLock;

use super::fs::SysFs;
use crate::{
    fs::utils::{
        Extension, FileSystem, Inode, InodeMode, InodeType, Metadata,
        systree_inode::{SysTreeInodeTy, SysTreeNodeKind},
    },
    prelude::*,
};

/// An inode abstraction used in the sysfs file system.
pub(super) struct SysFsInode {
    /// The corresponding node in the SysTree.
    node_kind: SysTreeNodeKind,
    /// The metadata of this inode.
    ///
    /// Most of the metadata (e.g., file size, timestamps)
    /// can be determined upon the creation of an inode,
    /// and are thus kept intact inside the immutable `metadata` field.
    /// Currently, the only mutable metadata is `mode`,
    /// which allows user space to `chmod` an inode on sysfs.
    metadata: Metadata,
    /// The extension of this inode.
    extension: Extension,
    /// The file mode (permissions) of this inode, protected by a lock.
    mode: RwLock<InodeMode>,
    /// Weak reference to the parent inode.
    parent: Weak<SysFsInode>,
    /// Weak self-reference for cyclic data structures.
    this: Weak<SysFsInode>,
}

impl SysTreeInodeTy for SysFsInode {
    fn new_arc(
        node_kind: SysTreeNodeKind,
        metadata: Metadata,
        mode: InodeMode,
        parent: Weak<Self>,
    ) -> Arc<Self>
    where
        Self: Sized,
    {
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

impl Inode for SysFsInode {
    fn fs(&self) -> Arc<dyn FileSystem> {
        SysFs::singleton().clone()
    }

    fn create(&self, _name: &str, _type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Err(Error::with_message(
            Errno::EPERM,
            "file creation under sysfs is not allowed",
        ))
    }
}
