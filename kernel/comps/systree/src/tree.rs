// SPDX-License-Identifier: MPL-2.0

//! Defines the main `SysTree` structure and its root node implementation.

use alloc::sync::{Arc, Weak};

use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

use super::{
    attr::SysAttrSet,
    node::{SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj},
    Error, Result, SysStr,
};
use crate::{impl_cast_methods_for_branch, SysBranchNodeFields};

#[derive(Debug)]
pub struct SysTree {
    root: Arc<RootNode>,
    // event_hub: SysEventHub,
}

impl SysTree {
    /// Creates a new `SysTree` instance with a default root node
    /// and standard subdirectories like "devices", "block", "kernel".
    /// This is intended to be called once for the singleton.
    pub(crate) fn new() -> Self {
        let name = ""; // Only the root has an empty name
        let attr_set = SysAttrSet::new_empty(); // The root has no attributes

        let root_node = Arc::new_cyclic(|weak_self| {
            let fields = SysBranchNodeFields::new(SysStr::from(name), attr_set, weak_self.clone());
            RootNode { fields }
        });

        Self { root: root_node }
    }

    /// Returns a reference to the root node of the tree.
    pub fn root(&self) -> &Arc<RootNode> {
        &self.root
    }
}

/// The root node in the `SysTree`.
///
/// A `RootNode` can work like a branching node, allowing to add additional nodes
/// as its children.
#[derive(Debug)]
pub struct RootNode {
    fields: SysBranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl RootNode {
    /// Adds a child node to this `RootNode`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

#[inherit_methods(from = "self.fields")]
impl SysObj for RootNode {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn is_root(&self) -> bool {
        true
    }

    fn init_parent(&self, _parent: Weak<dyn SysBranchNode>) {
        // This method should be a no-op for `RootNode`.
    }

    fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;
}

impl SysNode for RootNode {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for RootNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;
}
