// SPDX-License-Identifier: MPL-2.0

//! Defines the main `SysTree` structure and its root node implementation.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::any::Any;

use ostd::mm::{VmReader, VmWriter};
use spin::RwLock;

use super::{
    attr::SysAttrSet,
    node::{SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj},
    Error, Result, SysStr,
};

// --- SysTree Singleton Container ---

/// The main container for the system tree.
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
        let root_node = Arc::new(RootNode {
            id: SysNodeId::new(),
            name: "".into(),
            attrs: SysAttrSet::new(),
            children: RwLock::new(BTreeMap::new()),
        });

        // Add standard directories (using the same RootNode struct for simplicity for now)
        // TODO: Implement proper directory node types if needed.
        let devices_node = Arc::new(RootNode {
            id: SysNodeId::new(),
            name: "devices".into(),
            attrs: SysAttrSet::new(),
            children: RwLock::new(BTreeMap::new()),
        });
        root_node.add_child(devices_node).unwrap();

        let block_node = Arc::new(RootNode {
            id: SysNodeId::new(),
            name: "block".into(),
            attrs: SysAttrSet::new(),
            children: RwLock::new(BTreeMap::new()),
        });
        root_node.add_child(block_node).unwrap();

        let block_node = Arc::new(RootNode {
            id: SysNodeId::new(),
            name: "bus".into(),
            attrs: SysAttrSet::new(),
            children: RwLock::new(BTreeMap::new()),
        });
        root_node.add_child(block_node).unwrap();

        let kernel_node = Arc::new(RootNode {
            id: SysNodeId::new(),
            name: "kernel".into(),
            attrs: SysAttrSet::new(),
            children: RwLock::new(BTreeMap::new()),
        });
        root_node.add_child(kernel_node).unwrap();

        Self { root: root_node }
    }

    /// Returns a reference to the root node of the tree.
    /// Note: Returns the concrete `RootNode` type, not `dyn SysBranchNode`.
    pub fn root(&self) -> &Arc<RootNode> {
        &self.root
    }

    // Event methods removed as SysEventHub is not defined/included yet.
    // pub fn register_observer(...)
    // pub fn unregister_observer(...)
    // pub fn publish_event(...)
}

#[derive(Debug)]
pub struct RootNode {
    id: SysNodeId,
    name: SysStr,
    attrs: SysAttrSet,
    children: RwLock<BTreeMap<SysStr, Arc<dyn SysObj>>>,
}

impl RootNode {
    /// Adds a child node. This was part of the concrete RootNode impl in lib.rs.
    /// It's not part of the SysBranchNode trait definition.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()> {
        let name = new_child.name();
        let mut children_guard = self.children.write();
        if children_guard.contains_key(&name) {
            return Err(Error);
        }
        children_guard.insert(name.clone(), new_child);
        Ok(())
    }
}

impl SysObj for RootNode {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_node(&self) -> Option<&dyn super::node::SysNode> {
        Some(self)
    }
    fn as_branch(&self) -> Option<&dyn super::node::SysBranchNode> {
        Some(self)
    }
    fn arc_as_symlink(&self) -> Option<alloc::sync::Arc<dyn super::node::SysSymlink>> {
        None
    }
    fn arc_as_node(&self) -> Option<alloc::sync::Arc<dyn super::node::SysNode>> {
        None
    }
    fn arc_as_branch(&self) -> Option<alloc::sync::Arc<dyn super::node::SysBranchNode>> {
        None
    }

    // --- Methods required by the SysObj trait ---
    fn id(&self) -> &SysNodeId {
        &self.id
    }

    fn type_(&self) -> SysNodeType {
        SysNodeType::Branch
    }

    fn name(&self) -> SysStr {
        self.name.clone()
    }

    fn parent(&self) -> Option<Weak<dyn SysBranchNode>> {
        None
    }

    fn is_root(&self) -> bool {
        true
    }

    fn path(&self) -> String {
        "/".to_string()
    }
}

impl SysNode for RootNode {
    // --- Methods from original asterinas lib.rs RootNode impl ---
    fn node_attrs(&self) -> &SysAttrSet {
        &self.attrs
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<()> {
        Err(Error)
    }

    // show_attr and store_attr default implementations in the trait should suffice if read/write are implemented.
    // We can override them if specific formatting is needed.
    // fn show_attr(&self, name: &str) -> Result<String> { ... }
    // fn store_attr(&self, name: &str, new_val: &str) -> Result<()> { ... }
}

impl SysBranchNode for RootNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&dyn SysNode>)) {
        let children_guard = self.children.read();
        let child_opt = children_guard.get(name);
        // We have Option<&Arc<dyn SysObj>>, need Option<&dyn SysNode>
        // Use the as_node() method as suggested in the comment below.
        // If downcast fails, it means the child is not a SysNode (e.g., a Symlink),
        // or the type doesn't match. The trait signature expects &dyn SysNode.
        // Let's refine this: SysBranchNode holds SysObj, visitor expects SysNode.
        // Maybe the visitor should accept Option<&dyn SysObj>?
        // Sticking to the trait: only pass if it IS a SysNode.
        // Need SysObj::as_node() method. Assuming it exists on the SysObj trait.
        let node_opt = child_opt.and_then(|obj_arc| obj_arc.as_node());
        f(node_opt);
    }

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&dyn SysObj) -> Option<()>) {
        // TODO: Implement min_id filtering if necessary. For now, iterate all.
        let children_guard = self.children.read();
        for child_arc in children_guard.values() {
            if f(child_arc.as_ref()).is_none() {
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
