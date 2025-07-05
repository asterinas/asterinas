// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use aster_systree::{
    impl_cast_methods_for_branch, Error, Result, SysAttrSet, SysBranchNode, SysBranchNodeFields,
    SysNode, SysNodeId, SysNodeType, SysObj, SysPerms, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};
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

/// Manager for virtual filesystem.
///
/// Represents the root of all filesystem-related sysfs entries, where:
/// - Each child node corresponds to a filesystem type (ext2, cgroup, etc.).
/// - Manages registration and lifetime of filesystem control interfaces.
#[derive(Debug)]
pub struct FsManager {
    fields: SysBranchNodeFields<dyn FsFactory>,
    weak_self: Weak<Self>,
}

/// Trait representing a filesystem's control interface.
pub trait FsFactory: SysObj {
    /// Called when the interface is mounted in sysfs.
    fn on_mount(&self);

    /// Called when the interface is unmounted from sysfs.  
    fn on_unmount(&self);
}

impl FsManager {
    fn new() -> Arc<Self> {
        let fields = SysBranchNodeFields::new(SysStr::from("fs"), SysAttrSet::new_empty());
        Arc::new_cyclic(|weak_self| FsManager {
            fields,
            weak_self: weak_self.clone(),
        })
    }

    /// Gets the `FsFactory` given the input `name`.
    pub fn get(&self, name: &str) -> Option<Arc<dyn FsFactory>> {
        self.fields.child(name)
    }

    /// Registers a filesystem control interface.
    pub fn register(&self, fs_factory: Arc<dyn FsFactory>) -> crate::Result<()> {
        fs_factory.on_mount();
        self.fields.add_child(fs_factory).map_err(|e| e.into())
    }

    /// Unregisters a filesystem control interface.
    pub fn unregister(&self, name: &str) -> crate::Result<()> {
        let fs_factory = self.fields.remove_child(name).map_err(crate::Error::from)?;
        fs_factory.on_unmount();
        Ok(())
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for FsManager {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn init_parent_path(&self, path: SysStr);

    fn parent_path(&self) -> Option<&SysStr>;
}

impl SysNode for FsManager {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
}

impl SysBranchNode for FsManager {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>)) {
        let children_guard = self.fields.children.read();
        let child = children_guard
            .get(name)
            .map(|child| child.clone() as Arc<dyn SysObj>);
        f(child.as_ref())
    }

    fn visit_children_with(&self, min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>) {
        let children_guard = self.fields.children.read();
        for child_arc in children_guard.values() {
            if child_arc.id().as_u64() < min_id {
                continue;
            }

            let child = child_arc.clone() as Arc<dyn SysObj>;
            if f(&child).is_none() {
                break;
            }
        }
    }

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>> {
        self.fields
            .child(name)
            .map(|child| child as Arc<dyn SysObj>)
    }
}
