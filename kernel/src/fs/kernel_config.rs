// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, Error, Result, SysAttrSetBuilder, SysBranchNode,
    SysObj, SysPerms, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};
use spin::Once;

/// Registers a new kernel configuration object.
pub fn register(config_obj: Arc<dyn SysObj>) -> Result<()> {
    KERNEL_CONFIG_SINGLETON
        .get()
        .unwrap()
        .add_child(config_obj)?;
    Ok(())
}

/// Unregisters a kernel configuration object.
pub fn unregister(name: &str) -> Result<Arc<dyn SysObj>> {
    let child = KERNEL_CONFIG_SINGLETON.get().unwrap().remove_child(name)?;
    Ok(child)
}

pub(super) fn init() {
    KERNEL_CONFIG_SINGLETON.call_once(|| {
        let singleton = KernelConfig::new();
        aster_systree::singleton()
            .root()
            .add_child(singleton.clone())
            .unwrap();
        singleton
    });
}

static KERNEL_CONFIG_SINGLETON: Once<Arc<KernelConfig>> = Once::new();

/// A systree node that manages kernel configuration objects.
#[derive(Debug)]
pub struct KernelConfig {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl KernelConfig {
    fn new() -> Arc<Self> {
        let name = SysStr::from("kernel");
        let builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed.
        let attrs = builder.build().expect("Failed to build attribute set");
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            KernelConfig { fields }
        })
    }

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

inherit_sys_branch_node!(KernelConfig, fields, {
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
