// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use aster_systree::{
    impl_cast_methods_for_branch, Error, Result, SysAttrSet, SysAttrSetBuilder, SysBranchNode,
    SysBranchNodeFields, SysMode, SysNode, SysNodeId, SysNodeType, SysObj, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};
use spin::Once;

/// A systree node that represents the `/kernel` directory in the sysfs.
#[derive(Debug)]
pub struct KernelDirNode {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
}

impl KernelDirNode {
    fn new() -> Arc<Self> {
        let name = SysStr::from("kernel");

        let builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed.
        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysBranchNodeFields::new(name, attrs);
        Arc::new_cyclic(|weak_self| KernelDirNode {
            fields,
            weak_self: weak_self.clone(),
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for KernelDirNode {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn set_parent_path(&self, path: SysStr);

    fn path(&self) -> SysStr;
}

impl SysNode for KernelDirNode {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        // TODO: Add support for reading attributes.
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        // TODO: Add support for writing attributes.
        Err(Error::AttributeError)
    }

    fn mode(&self) -> SysMode {
        SysMode::DEFAULT_RW_MODE
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for KernelDirNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

static KERNEL_DIR_SINGLETON: Once<Arc<KernelDirNode>> = Once::new();

/// Returns a reference to the node instance that corresponds to `/sys/kernel`. Panics if not initialized.
pub fn singleton() -> &'static Arc<KernelDirNode> {
    KERNEL_DIR_SINGLETON
        .get()
        .expect("the node corresponding to `/sys/kernel` is not initialized")
}

pub(super) fn init() {
    let kernel_dir = KernelDirNode::new();

    // Initialize the singleton.
    KERNEL_DIR_SINGLETON.call_once(|| kernel_dir);
}
