// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};
use core::fmt::Debug;

use aster_systree::{
    impl_cast_methods_for_branch, Error, Result, SysAttrSet, SysBranchNode, SysBranchNodeFields,
    SysMode, SysNode, SysNodeId, SysNodeType, SysObj, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

/// The systree node that represents the root node of the configfs.
#[derive(Debug)]
pub struct ConfigRootNode {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
}

impl ConfigRootNode {
    pub(super) fn new() -> Arc<Self> {
        let name = SysStr::from("config");

        let attrs = SysAttrSet::new_empty();
        let fields = SysBranchNodeFields::new(name, attrs);
        Arc::new_cyclic(|weak_self| ConfigRootNode {
            fields,
            weak_self: weak_self.clone(),
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for ConfigRootNode {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn is_root(&self) -> bool {
        true
    }

    fn path(&self) -> SysStr {
        SysStr::from("/")
    }
}

impl SysNode for ConfigRootNode {
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
        SysMode::DEFAULT_RW_MODE
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for ConfigRootNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()> {
        let name = new_child.name();
        let mut children_guard = self.fields.children.write();
        if children_guard.contains_key(name) {
            return Err(Error::ChildExisted);
        }

        new_child.set_parent_path(SysStr::from(""));
        children_guard.insert(name.clone(), new_child);
        Ok(())
    }
}
