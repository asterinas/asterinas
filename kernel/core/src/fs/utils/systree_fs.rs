// SPDX-License-Identifier: MPL-2.0

//! VFS support for singleton file systems backed by a `SysTree` model.

use aster_systree::{SysBranchNode, SysNode};
use spin::Once;

use crate::{
    fs::{
        file::InodeMode,
        pseudofs::AnonDeviceId,
        utils::systree_inode::{SysTreeInodeTy, SysTreeNodeKind},
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
            inode::{Extension, Inode, Metadata},
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
};

/// A type descriptor for a singleton file system backed by a fixed `SysTree` root.
///
/// The resulting file system uses the default [`SysTreeInodeTy`] mutation
/// behavior: create requests are forwarded to the model, while removal and
/// rename requests are rejected. File systems with per-mount roots or custom
/// mutation and revalidation semantics require dedicated implementations.
pub(in crate::fs) struct SingletonSysTreeFsType {
    name: &'static str,
    magic: u64,
    block_size: usize,
    name_max: usize,
    root: fn() -> Arc<dyn SysBranchNode>,
    instance: Once<Arc<SysTreeFileSystem>>,
}

impl SingletonSysTreeFsType {
    pub(in crate::fs) const fn new(
        name: &'static str,
        magic: u64,
        block_size: usize,
        name_max: usize,
        root: fn() -> Arc<dyn SysBranchNode>,
    ) -> Self {
        Self {
            name,
            magic,
            block_size,
            name_max,
            root,
            instance: Once::new(),
        }
    }

    pub(in crate::fs) fn singleton(&self) -> &Arc<SysTreeFileSystem> {
        self.instance.call_once(|| {
            SysTreeFileSystem::new(
                self.name,
                self.magic,
                self.block_size,
                self.name_max,
                (self.root)(),
            )
        })
    }
}

impl FsType for SingletonSysTreeFsType {
    type Key = ();

    fn name(&self) -> &'static str {
        self.name
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, _fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        Ok(self.singleton().clone())
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}

/// A file system that presents a `SysTree` through the VFS.
pub(in crate::fs) struct SysTreeFileSystem {
    name: &'static str,
    _anon_device_id: AnonDeviceId,
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

impl SysTreeFileSystem {
    fn new(
        name: &'static str,
        magic: u64,
        block_size: usize,
        name_max: usize,
        root_node: Arc<dyn SysBranchNode>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let anon_device_id = AnonDeviceId::acquire()
                .expect("no device ID is available for the SysTree file system");
            let sb = SuperBlock::new(magic, block_size, name_max, anon_device_id.id());
            let weak_fs: Weak<dyn FileSystem> = weak_self.clone();
            let root = SysTreeInode::new_root(root_node, &sb, weak_fs);

            Self {
                name,
                _anon_device_id: anon_device_id,
                sb,
                root,
                fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            }
        })
    }
}

impl FileSystem for SysTreeFileSystem {
    fn name(&self) -> &'static str {
        self.name
    }

    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        self.sb.clone()
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

struct SysTreeInode {
    node_kind: SysTreeNodeKind,
    metadata: Metadata,
    extension: Extension,
    mode: RwLock<InodeMode>,
    parent: Weak<Self>,
    fs: Weak<dyn FileSystem>,
    this: Weak<Self>,
}

impl SysTreeInodeTy for SysTreeInode {
    fn new_arc(
        node_kind: SysTreeNodeKind,
        metadata: Metadata,
        mode: InodeMode,
        parent: Weak<Self>,
        fs: Weak<dyn FileSystem>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            node_kind,
            metadata,
            extension: Extension::new(),
            mode: RwLock::new(mode),
            parent,
            fs,
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

    fn extension(&self) -> &Extension {
        &self.extension
    }

    fn parent(&self) -> &Weak<Self> {
        &self.parent
    }

    fn this(&self) -> Arc<Self> {
        self.this
            .upgrade()
            .expect("invalid weak reference to `self`")
    }

    fn fs_weak(&self) -> &Weak<dyn FileSystem> {
        &self.fs
    }
}
