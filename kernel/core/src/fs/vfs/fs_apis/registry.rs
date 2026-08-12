// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::Entry;

use aster_block::BlockDevice;
use aster_systree::{
    AttrLessBranchNodeFields, SysNode, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
};
use spin::Once;

use crate::{
    fs::{
        fs_impls::sysfs,
        vfs::{
            file_system::{FileSystem, FsFlags},
            path::{AT_FDCWD, Dentry, EmptyPathStr, FsPath},
        },
    },
    prelude::*,
};

/// A type of file system.
pub(crate) trait FsType: Send + Sync + 'static {
    /// Key used to deduplicate mounts of the same logical FS instance.
    ///
    /// Set to `()` (with the default `obtain_key_and_cache` returning `None`) to opt out
    /// of any caching — every mount is fresh.
    type Key: Eq + Ord + Clone + Send + Sync + 'static;

    /// Gets the name of this FS type such as `"ext4"` or `"sysfs"`.
    fn name(&self) -> &'static str;

    /// Gets the properties of this FS type.
    fn properties(&self) -> FsProperties;

    /// Creates an instance of this FS type.
    fn create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>>;

    /// Returns the computed dedup key and the [`FsCache`] for this mount request.
    ///
    /// Return `None` to skip the cache mechanism (every mount is fresh).
    fn obtain_key_and_cache(
        &self,
        _fs_creation_ctx: &mut FsCreationCtx,
    ) -> Option<(Self::Key, &FsCache<Self::Key>)> {
        None
    }

    /// Returns a `SysTree` node that represents the FS type.
    ///
    /// If a FS type is not intended to appear under SysFs,
    /// then this method returns a `None`.
    ///
    /// The same result will be returned by this method
    /// when it is called multiple times.
    fn sysnode(&self) -> Option<Arc<dyn SysNode>>;
}

/// Object-safe view of [`FsType`] used by the registry, mount syscall, and procfs.
///
/// Implemented automatically for every [`FsType`] via a blanket impl, so FS
/// authors only implement [`FsType`].
pub(crate) trait DynFsType: Send + Sync + 'static {
    /// Gets the name of this FS type such as `"ext4"` or `"sysfs"`.
    fn name(&self) -> &'static str;

    /// Gets the properties of this FS type.
    fn properties(&self) -> FsProperties;

    /// Gets or creates a file system instance along with its root dentry.
    fn get_or_create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<FsAndRoot>;

    /// Returns a `SysTree` node that represents the FS type.
    fn sysnode(&self) -> Option<Arc<dyn SysNode>>;
}

impl<T: FsType> DynFsType for T {
    fn name(&self) -> &'static str {
        <T as FsType>::name(self)
    }

    fn properties(&self) -> FsProperties {
        <T as FsType>::properties(self)
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        <T as FsType>::sysnode(self)
    }

    fn get_or_create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<FsAndRoot> {
        if let Some((key, cache)) = self.obtain_key_and_cache(fs_creation_ctx) {
            cache.get_or_create(key, fs_creation_ctx.flags(), || {
                self.create(fs_creation_ctx)
            })
        } else {
            let fs = self.create(fs_creation_ctx)?;
            Ok(FsAndRoot::new(fs))
        }
    }
}

/// A context that describes the inputs used to create a file system instance.
///
/// This context is used to identify and create a file system instance.
pub(crate) struct FsCreationCtx<'a> {
    source: Option<&'a str>,
    flags: FsFlags,
    args: Option<&'a str>,
    block_device: BlockDeviceResolution<'a>,
}

enum BlockDeviceResolution<'a> {
    Pending(&'a Context<'a>),
    Resolved(Arc<dyn BlockDevice>),
}

impl<'a> FsCreationCtx<'a> {
    /// Creates a file system creation context from syscall inputs.
    pub(crate) fn new(
        source: Option<&'a str>,
        flags: FsFlags,
        args: Option<&'a str>,
        task_ctx: &'a Context<'a>,
    ) -> Self {
        Self {
            source,
            flags,
            args,
            block_device: BlockDeviceResolution::Pending(task_ctx),
        }
    }

    /// Creates a file system creation context from an already resolved block device.
    pub(in crate::fs) fn from_block_device(
        block_device: Arc<dyn BlockDevice>,
        flags: FsFlags,
        args: Option<&'a str>,
    ) -> Self {
        Self {
            source: None,
            flags,
            args,
            block_device: BlockDeviceResolution::Resolved(block_device),
        }
    }

    /// Returns the user-supplied mount source.
    pub(in crate::fs) fn source(&self) -> Option<&str> {
        self.source
    }

    /// Returns the user-supplied mount flags.
    pub(in crate::fs) fn flags(&self) -> FsFlags {
        self.flags
    }

    /// Returns the file system specific mount arguments.
    pub(in crate::fs) fn args(&self) -> Option<&str> {
        self.args
    }

    /// Resolves the mount source into a block device.
    pub(in crate::fs) fn resolve_block_device(&mut self) -> Result<&Arc<dyn BlockDevice>> {
        if self.block_device().is_none() {
            self.do_resolve_block_device()?;
        }

        Ok(self.block_device().unwrap())
    }

    fn block_device(&self) -> Option<&Arc<dyn BlockDevice>> {
        match &self.block_device {
            BlockDeviceResolution::Pending(_) => None,
            BlockDeviceResolution::Resolved(block_device) => Some(block_device),
        }
    }

    fn do_resolve_block_device(&mut self) -> Result<()> {
        let task_ctx = match &self.block_device {
            BlockDeviceResolution::Pending(task_ctx) => *task_ctx,
            BlockDeviceResolution::Resolved(_) => return Ok(()),
        };

        let source = self
            .source()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the source is not specified"))?;
        let fs_path = FsPath::from_fd_at(AT_FDCWD, source, EmptyPathStr::Reject)?;
        let path = task_ctx
            .thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?;

        if !path.type_().is_device() {
            return_errno_with_message!(Errno::ENODEV, "the path is not a device file");
        }
        let id = path.metadata()?.self_dev_id;

        let block_device = id
            .and_then(aster_block::lookup)
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "the device is not found"))?;
        self.block_device = BlockDeviceResolution::Resolved(block_device);

        Ok(())
    }
}

bitflags! {
    /// The properties common to all FS instances.
    pub(crate) struct FsProperties: u32 {
        /// Whether a FS needs to be backed by a disk.
        ///
        /// Most persistent FSes such as ext2 require disks.
        /// But a volatile FS such as ramfs or
        /// a pseudo FS such as sysfs does not.
        const NEED_DISK = 1 << 1;
    }
}

/// Registers a new FS type.
//
// TODO: Figure out what should happen when unregistering the FS type.
pub(crate) fn register(new_type: &'static dyn DynFsType) -> Result<()> {
    FS_REGISTRY.get().unwrap().register(new_type)
}

/// Looks up a FS type.
pub(crate) fn look_up(name: &str) -> Option<&'static dyn DynFsType> {
    FS_REGISTRY
        .get()
        .unwrap()
        .fs_table
        .lock()
        .get(name)
        .cloned()
}

/// Executes a user-provided operation with an iterator that can access each
/// and every FS type.
pub(crate) fn with_iter<F, R>(f: F) -> R
where
    F: FnOnce(&mut dyn Iterator<Item = (&str, &dyn DynFsType)>) -> R,
{
    let guard = FS_REGISTRY.get().unwrap().fs_table.lock();

    let mut iter = guard.iter().map(|(name, fs_type)| (*name, *fs_type));
    f(&mut iter)
}

/// Initializes the FS registry module.
pub(crate) fn init() {
    // This object will appear at the `/sys/fs` path
    FS_REGISTRY.call_once(|| {
        let singleton = FsRegistry::new();
        sysfs::systree_singleton()
            .root()
            .add_child(singleton.clone())
            .unwrap();
        singleton
    });
}

static FS_REGISTRY: Once<Arc<FsRegistry>> = Once::new();

/// A cache of file system instances, keyed by `K`.
///
/// `FsType` implementations use this cache to deduplicate mounts that refer to
/// the same logical file system instance, such as the same block device.
pub(crate) struct FsCache<K: Eq + Ord + Clone + Send + Sync + 'static> {
    entries: Mutex<BTreeMap<K, WeakFsAndRoot>>,
}

/// A file system paired with its root [`Dentry`].
#[derive(Clone)]
pub(crate) struct FsAndRoot {
    fs: Arc<dyn FileSystem>,
    root_dentry: Arc<Dentry>,
}

impl FsAndRoot {
    /// Creates an `FsAndRoot` by deriving the root dentry from the file system.
    ///
    /// This root is the file system root created from [`FileSystem::root_inode`].
    pub(crate) fn new(fs: Arc<dyn FileSystem>) -> Self {
        let root_dentry = Dentry::new_root(fs.root_inode());
        Self { fs, root_dentry }
    }

    /// Consumes the `FsAndRoot` and returns the file system and its root dentry.
    pub(in crate::fs) fn into_parts(self) -> (Arc<dyn FileSystem>, Arc<Dentry>) {
        (self.fs, self.root_dentry)
    }

    /// Returns a reference the file system.
    pub(in crate::fs) fn fs(&self) -> &Arc<dyn FileSystem> {
        &self.fs
    }

    fn downgrade(&self) -> WeakFsAndRoot {
        WeakFsAndRoot {
            fs: Arc::downgrade(&self.fs),
            root_dentry: Arc::downgrade(&self.root_dentry),
        }
    }
}

struct WeakFsAndRoot {
    fs: Weak<dyn FileSystem>,
    root_dentry: Weak<Dentry>,
}

impl WeakFsAndRoot {
    fn upgrade(&mut self) -> Option<FsAndRoot> {
        let fs = self.fs.upgrade()?;
        let root_dentry = self.root_dentry.upgrade().unwrap_or_else(|| {
            let dentry = Dentry::new_root(fs.root_inode());
            self.root_dentry = Arc::downgrade(&dentry);
            dentry
        });
        Some(FsAndRoot { fs, root_dentry })
    }
}

impl<K: Eq + Ord + Clone + Send + Sync + 'static> FsCache<K> {
    /// Creates an empty cache.
    pub(in crate::fs) const fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Gets or creates a file system and its root dentry for `key`.
    ///
    /// A live entry is checked against `required_flags` while the cache is locked and
    /// returned without invoking `create_fn`. If no live entry exists, invokes
    /// `create_fn` and derives a root `Dentry`.
    fn get_or_create<F>(&self, key: K, required_flags: FsFlags, create_fn: F) -> Result<FsAndRoot>
    where
        F: FnOnce() -> Result<Arc<dyn FileSystem>>,
    {
        let mut entries = self.entries.lock();
        match entries.entry(key) {
            Entry::Occupied(mut entry) => {
                if let Some(fs_and_root) = entry.get_mut().upgrade() {
                    let extant_readonly = fs_and_root.fs.flags().contains(FsFlags::RDONLY);
                    let requested_readonly = required_flags.contains(FsFlags::RDONLY);
                    if extant_readonly != requested_readonly {
                        return_errno_with_message!(
                            Errno::EBUSY,
                            "the read-only flag of the extant file system does not match"
                        );
                    }
                    return Ok(fs_and_root);
                }

                let fs = match create_fn() {
                    Ok(fs) => fs,
                    Err(err) => {
                        entry.remove();
                        return Err(err);
                    }
                };
                let fs_and_root = FsAndRoot::new(fs);
                entry.insert(fs_and_root.downgrade());
                Ok(fs_and_root)
            }
            Entry::Vacant(entry) => {
                let fs = create_fn()?;
                let fs_and_root = FsAndRoot::new(fs);
                entry.insert(fs_and_root.downgrade());
                Ok(fs_and_root)
            }
        }
    }
}

struct FsRegistry {
    fs_table: Mutex<BTreeMap<&'static str, &'static dyn DynFsType>>,
    systree_fields: AttrLessBranchNodeFields<dyn SysObj, Self>,
}

impl Debug for FsRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FsRegistry")
            .field("systree_fields", &self.systree_fields)
            .finish()
    }
}

impl FsRegistry {
    fn new() -> Arc<Self> {
        let name = SysStr::from("fs");
        Arc::new_cyclic(|weak_self| {
            let fs_table = Mutex::new(BTreeMap::new());
            let systree_fields = AttrLessBranchNodeFields::new(name, weak_self.clone());
            Self {
                fs_table,
                systree_fields,
            }
        })
    }

    /// Registers a file system control interface.
    fn register(&self, new_type: &'static dyn DynFsType) -> Result<()> {
        let mut fs_table = self.fs_table.lock();
        if fs_table.contains_key(new_type.name()) {
            return_errno_with_message!(Errno::EEXIST, "the file system type already exists");
        }

        if let Some(node) = new_type.sysnode() {
            self.systree_fields.add_child(node)?;
        }

        fs_table.insert(new_type.name(), new_type);
        Ok(())
    }
}

inherit_sys_branch_node!(FsRegistry, systree_fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});
