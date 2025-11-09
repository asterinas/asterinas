// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, Error, Result, SysAttrSetBuilder, SysBranchNode,
    SysNode, SysPerms, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};
use spin::Once;

/// Registers a new kernel `SysNode`.
pub(super) fn register(config_obj: Arc<dyn SysNode>) -> crate::prelude::Result<()> {
    KERNEL_SYS_NODE_ROOT.get().unwrap().add_child(config_obj)?;
    Ok(())
}

/// Unregisters a kernel `SysNode`.
pub(super) fn unregister(name: &str) -> crate::prelude::Result<()> {
    let _ = KERNEL_SYS_NODE_ROOT.get().unwrap().remove_child(name)?;
    Ok(())
}

pub(super) fn init() {
    KERNEL_SYS_NODE_ROOT.call_once(|| {
        let singleton = KernelSysNodeRoot::new();
        super::systree_singleton()
            .root()
            .add_child(singleton.clone())
            .unwrap();

        singleton
    });
}

static KERNEL_SYS_NODE_ROOT: Once<Arc<KernelSysNodeRoot>> = Once::new();

/// A systree node representing the `/sys/kernel` directory.
///
/// This node serves as the root for all kernel-related sysfs entries,
/// including kernel parameters, debugging interfaces, and various
/// kernel subsystem information. It corresponds to the `/kernel`
/// directory in the sysfs filesystem.
#[derive(Debug)]
pub struct KernelSysNodeRoot {
    fields: BranchNodeFields<dyn SysNode, Self>,
}

#[inherit_methods(from = "self.fields")]
impl KernelSysNodeRoot {
    /// Creates a new `KernelSysNodeRoot` instance.
    fn new() -> Arc<Self> {
        let name = SysStr::from("kernel");
        let builder = SysAttrSetBuilder::new();
        // TODO: Add more kernel-specific attributes.
        let attrs = builder
            .build()
            .expect("Failed to build kernel attribute set");
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            KernelSysNodeRoot { fields }
        })
    }

    /// Adds a kernel `SysNode` to this node.
    fn add_child(&self, new_child: Arc<dyn SysNode>) -> Result<()>;
}

inherit_sys_branch_node!(KernelSysNodeRoot, fields, {
    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        // TODO: Add support for reading attributes.
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        // TODO: Add support for writing attributes.
        Err(Error::AttributeError)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});
