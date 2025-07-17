// SPDX-License-Identifier: MPL-2.0

use alloc::borrow::ToOwned;

use aster_block::BlockDevice;
use aster_systree::{
    inherit_sys_branch_node, AttrLessBranchNodeFields, SysBranchNode, SysObj, SysPerms, SysStr,
};
use spin::Once;

use crate::{fs::utils::FileSystem, prelude::*};

/// A type of file system.
pub trait FsType: Send + Sync + 'static {
    /// Gets the name of this FS type such as `"ext4"` or `"sysfs"`.
    fn name(&self) -> &'static str;

    /// Gets the properties of this FS type.
    fn properties(&self) -> FsProperties;

    /// Creates an instance of this FS type.
    ///
    /// The optional `disk` argument must be provided
    /// if `self.properties()` contains `FsProperties::NEED_DISK`.
    fn create(
        &self,
        args: Option<CString>,
        disk: Option<Arc<dyn BlockDevice>>,
        ctx: &Context,
    ) -> Result<Arc<dyn FileSystem>>;

    /// Returns a `SysTree` node that represents the FS type.
    ///
    /// If a FS type is not intended to appear under SysFs,
    /// then this method returns a `None`.
    ///
    /// The same result will be returned by this method
    /// when it is called multiple times.
    fn sysnode(&self) -> Option<Arc<dyn SysBranchNode>>;
}

bitflags! {
    /// The properties common to all FS instances.
    pub struct FsProperties: u32 {
        /// Whether a FS needs to be backed by a disk.
        ///
        /// Most persistent FSes such as Ext2 require disks.
        /// But a volatile FS such as RamFS or
        /// a pseudo FS such as SysFS does not.
        const NEED_DISK = 1 << 1;
    }
}

/// Registers a new FS type.
pub fn register(new_type: Arc<dyn FsType>) -> Result<()> {
    FS_REGISTRY.get().unwrap().register(new_type)
}

/// Unregisters a new FS type.
pub fn unregister(name: &str) -> Result<Arc<dyn FsType>> {
    FS_REGISTRY.get().unwrap().unregister(name)
}

/// Looks up a FS type.
pub fn look_up(name: &str) -> Option<Arc<dyn FsType>> {
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
pub fn with_iter<F, R>(f: F) -> R
where
    F: FnOnce(&mut dyn Iterator<Item = (&String, &Arc<dyn FsType>)>) -> R,
{
    let guard = FS_REGISTRY.get().unwrap().fs_table.lock();
    let mut iter = guard.iter();

    f(&mut iter)
}

/// Initialize the FS registry module.
pub fn init() {
    // This object will appear at the `/sys/fs` path
    FS_REGISTRY.call_once(|| {
        let singleton = FsRegistry::new();
        aster_systree::singleton()
            .root()
            .add_child(singleton.clone())
            .unwrap();
        singleton
    });
}

static FS_REGISTRY: Once<Arc<FsRegistry>> = Once::new();

struct FsRegistry {
    fs_table: Mutex<BTreeMap<String, Arc<dyn FsType>>>,
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
    fn register(&self, new_type: Arc<dyn FsType>) -> crate::Result<()> {
        let mut fs_table = self.fs_table.lock();
        if fs_table.contains_key(new_type.name()) {
            return_errno_with_message!(Errno::EEXIST, "File system type already registered");
        }

        if let Some(node) = new_type.sysnode() {
            self.systree_fields.add_child(node)?;
        }

        fs_table.insert(new_type.name().to_owned(), new_type);
        Ok(())
    }

    /// Unregisters a file system control interface.
    fn unregister(&self, name: &str) -> crate::Result<Arc<dyn FsType>> {
        let _ = self.systree_fields.remove_child(name);

        self.fs_table
            .lock()
            .remove(name)
            .ok_or(Error::new(Errno::ENOENT))
    }
}

inherit_sys_branch_node!(FsRegistry, systree_fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});
