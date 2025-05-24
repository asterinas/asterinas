// SPDX-License-Identifier: MPL-2.0

//! Defines the main `SysTree` structure and its root node implementation.

use alloc::{
    string::ToString,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::any::Any;

use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

use super::{
    attr::SysAttrSet,
    node::{SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj},
    Error, Result, SysStr,
};
use crate::{SysAttrFlags, SysBranchNodeFields};

#[derive(Debug)]
pub struct SysTree {
    root: Arc<BasicSysNode>,
    // event_hub: SysEventHub,
}

impl SysTree {
    /// Creates a new `SysTree` instance with a default root node
    /// and standard subdirectories like "devices", "block", "kernel".
    /// This is intended to be called once for the singleton.
    pub(crate) fn new() -> Self {
        let name = ""; // Only the root has an empty name
        let attr_set = SysAttrSet::new_empty(); // The root has no attributes
        let fields = SysBranchNodeFields::new(name.to_string().into(), attr_set);

        let root_node = Arc::new_cyclic(|weak_self| BasicSysNode {
            fields,
            self_ref: weak_self.clone(),
        });

        Self { root: root_node }
    }

    /// Returns a reference to the root node of the tree.
    pub fn root(&self) -> &Arc<BasicSysNode> {
        &self.root
    }
}

/// The basic node used in the `SysTree`.
///
/// A `SysTree` defaults to using a BasicSysNode as its root node.
/// A `BasicSysNode` can work like a branching node, allowing to add additional nodes
/// as its children. When a `BasicSysNode` has no child nodes, it is treated as a leaf node.
#[derive(Debug)]
pub struct BasicSysNode {
    fields: SysBranchNodeFields,
    self_ref: Weak<Self>,
}

impl BasicSysNode {
    /// Adds a child node to this `BasicSysNode`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()> {
        let name = new_child.name();
        let mut children_guard = self.fields.children.write();
        if children_guard.contains_key(&name) {
            return Err(Error::PermissionDenied);
        }
        children_guard.insert(name.clone(), new_child);
        Ok(())
    }

    /// Creates a new basic normal child node with the given name and attribute set.
    pub fn create_normal_child(&self, name: SysStr, attr_set: SysAttrSet) -> Result<()> {
        let mut children_guard = self.fields.children.write();
        if children_guard.contains_key(&name) {
            return Err(Error::PermissionDenied);
        }

        let new_child = {
            let fields = SysBranchNodeFields::new(name.clone(), attr_set);
            Arc::new_cyclic(|weak_self| BasicSysNode {
                fields,
                self_ref: weak_self.clone(),
            })
        };

        children_guard.insert(name, new_child);
        Ok(())
    }
}

impl SysObj for BasicSysNode {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn arc_as_node(&self) -> Option<Arc<dyn SysNode>> {
        self.self_ref
            .upgrade()
            .map(|arc_self| arc_self as Arc<dyn SysNode>)
    }

    fn arc_as_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.self_ref
            .upgrade()
            .map(|arc_self| arc_self as Arc<dyn SysBranchNode>)
    }

    fn id(&self) -> &SysNodeId {
        self.fields.id()
    }

    fn type_(&self) -> SysNodeType {
        if self.fields.children.read().is_empty() {
            return SysNodeType::Leaf;
        }
        SysNodeType::Branch
    }

    fn name(&self) -> SysStr {
        self.fields.name().to_string().into()
    }

    fn is_root(&self) -> bool {
        true
    }
}

impl SysNode for BasicSysNode {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        self.node_attrs()
            .get(name)
            .ok_or(Error::AttributeError)
            .and_then(|attr| {
                if attr.flags().contains(SysAttrFlags::CAN_READ) {
                    attr.read_attr(writer)
                } else {
                    Err(Error::PermissionDenied)
                }
            })
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        self.node_attrs()
            .get(name)
            .ok_or(Error::AttributeError)
            .and_then(|attr| {
                if attr.flags().contains(SysAttrFlags::CAN_WRITE) {
                    attr.write_attr(reader)
                } else {
                    Err(Error::PermissionDenied)
                }
            })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for BasicSysNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&dyn SysNode>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    fn children(&self) -> Vec<Arc<dyn SysObj>>;
}
