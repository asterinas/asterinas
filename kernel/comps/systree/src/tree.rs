// SPDX-License-Identifier: MPL-2.0

//! Defines the main `SysTree` structure and its root node implementation.

use alloc::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::any::Any;

use ostd::{
    mm::{VmReader, VmWriter},
    sync::RwLock,
};

use super::{
    attr::SysAttrSet,
    node::{SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj, SysSymlink},
    Error, Result, SysStr,
};

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
        let root_node = Arc::new_cyclic(|weak_self| RootNode {
            id: SysNodeId::new(),
            name: "".into(),
            attrs: SysAttrSet::new_empty(),
            children: RwLock::new(BTreeMap::new()),
            self_ref: weak_self.clone(),
        });

        Self { root: root_node }
    }

    /// Returns a reference to the root node of the tree.
    pub fn root(&self) -> &Arc<RootNode> {
        &self.root
    }
}

#[derive(Debug)]
pub struct RootNode {
    id: SysNodeId,
    name: SysStr,
    attrs: SysAttrSet,
    children: RwLock<BTreeMap<SysStr, Arc<dyn SysObj>>>,
    self_ref: Weak<Self>,
}

impl RootNode {
    /// Adds a child node. This was part of the concrete RootNode impl in lib.rs.
    /// It's not part of the SysBranchNode trait definition.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()> {
        let name = new_child.name();
        let mut children_guard = self.children.write();
        if children_guard.contains_key(&name) {
            return Err(Error::PermissionDenied);
        }
        children_guard.insert(name.clone(), new_child);
        Ok(())
    }
}

impl SysObj for RootNode {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn arc_as_symlink(&self) -> Option<Arc<dyn SysSymlink>> {
        None
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
        &self.id
    }

    fn type_(&self) -> SysNodeType {
        if self.children.read().is_empty() {
            return SysNodeType::Leaf;
        }
        SysNodeType::Branch
    }

    fn name(&self) -> SysStr {
        self.name.clone()
    }

    fn is_root(&self) -> bool {
        true
    }

    fn path(&self) -> SysStr {
        Cow::from("/")
    }
}

impl SysNode for RootNode {
    fn node_attrs(&self) -> &SysAttrSet {
        &self.attrs
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }
}

impl SysBranchNode for RootNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&dyn SysNode>)) {
        let children_guard = self.children.read();
        children_guard
            .get(name)
            .map(|child| {
                if let Some(node_ref) = child.arc_as_node().as_deref() {
                    f(Some(node_ref));
                } else {
                    f(None);
                }
            })
            .unwrap_or_else(|| f(None));
    }

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>) {
        let children_guard = self.children.read();
        for child_arc in children_guard.values() {
            if f(child_arc).is_none() {
                break;
            }
        }
    }

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>> {
        self.children.read().get(name).cloned()
    }

    fn children(&self) -> Vec<Arc<dyn SysObj>> {
        self.children.read().values().cloned().collect()
    }
}
