// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{inherit_sys_branch_node, AttrLessBranchNodeFields, SysObj, SysPerms, SysStr};
use spin::Once;

/// Returns a reference to the [`FsManager`]. Panics if not initialized.
pub fn singleton() -> &'static Arc<FsManager> {
    MANAGER.get().expect("Fs manager has not been initialized")
}

/// Initializes the [`FsManager`] singleton
pub fn init() {
    MANAGER.call_once(FsManager::new);
    aster_systree::singleton()
        .root()
        .add_child(singleton().clone())
        .expect("Failed to add fs manager to SysTree");
}

static MANAGER: Once<Arc<FsManager>> = Once::new();

/// The system-wide manager for file systems.
///
/// This manager also represents the parent of all sysfs entries related to
/// file systems, where:
/// - Each child node corresponds to a file system type (ext2, cgroup, etc.).
/// - Manages registration and lifetime of file system control interfaces.
#[derive(Debug)]
pub struct FsManager {
    fields: AttrLessBranchNodeFields<dyn FsControl, Self>,
}

/// A trait to represent control interfaces of a file system.
///
/// TODO: Add the actual control methods.
pub trait FsControl: SysObj {}

inherit_sys_branch_node!(FsManager, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

impl FsManager {
    fn new() -> Arc<Self> {
        let name = SysStr::from("fs");
        Arc::new_cyclic(|weak_self| {
            let fields = AttrLessBranchNodeFields::new(name, weak_self.clone());
            FsManager { fields }
        })
    }

    /// Gets the [`FsControl`] given the input `name`.
    pub fn get(&self, name: &str) -> Option<Arc<dyn FsControl>> {
        self.fields.child(name)
    }

    /// Registers a file system control interface.
    pub fn register(&self, fs_factory: Arc<dyn FsControl>) -> crate::Result<()> {
        self.fields.add_child(fs_factory).map_err(|e| e.into())
    }

    /// Unregisters a file system control interface.
    pub fn unregister(&self, name: &str) -> crate::Result<()> {
        self.fields.remove_child(name).map_err(crate::Error::from)?;

        Ok(())
    }
}
