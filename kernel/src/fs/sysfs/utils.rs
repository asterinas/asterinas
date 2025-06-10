// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use aster_systree::{
    impl_cast_methods_for_branch, Error, Result, SysAttrSet, SysBranchNode, SysBranchNodeFields,
    SysMode, SysNode, SysNodeId, SysNodeType, SysObj, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

/// A basic branch node in the SysTree, representing a directory-like structure.
///
/// This node can have children but does not have any specific attributes or behaviors beyond
/// those defined in [`SysBranchNodeFields`].
#[derive(Debug)]
pub struct BasicBranchNode {
    fields: SysBranchNodeFields<dyn SysObj>,
    mode: SysMode,
    weak_self: Weak<Self>,
}

#[inherit_methods(from = "self.fields")]
impl BasicBranchNode {
    /// Creates a new `BasicBranchNode` with the given name and [`SysMode`].
    pub fn new(name: SysStr, mode: SysMode) -> Arc<Self> {
        let fields = SysBranchNodeFields::new(name, SysAttrSet::new_empty());
        Arc::new_cyclic(|weak_self| BasicBranchNode {
            fields,
            mode,
            weak_self: weak_self.clone(),
        })
    }

    /// Adds a child node to this `BasicBranchNode`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

#[inherit_methods(from = "self.fields")]
impl SysObj for BasicBranchNode {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn is_root(&self) -> bool {
        false
    }

    fn set_parent_path(&self, path: SysStr);

    fn path(&self) -> SysStr;
}

impl SysNode for BasicBranchNode {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn mode(&self) -> SysMode {
        self.mode
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for BasicBranchNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;
}
