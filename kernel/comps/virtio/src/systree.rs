// SPDX-License-Identifier: MPL-2.0

//! Virtio node registration to systree

extern crate alloc;

use alloc::{borrow::Cow, collections::BTreeMap, string::ToString, sync::Arc, vec::Vec};

use ostd::mm::{VmReader, VmWriter};
use spin::RwLock;
use systree::{
    Result, RootNode, SysAttrSet, SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj,
};

/// Virtio root node under systree
#[derive(Debug)]
pub struct VirtioNode {
    id: SysNodeId,
    name: alloc::string::String,
    attrs: SysAttrSet,
    children: RwLock<BTreeMap<alloc::string::String, Arc<dyn SysObj>>>,
}

impl VirtioNode {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            id: SysNodeId::new(),
            name: "virtio".to_string(),
            attrs: SysAttrSet::new(),
            children: RwLock::new(BTreeMap::new()),
        })
    }
}

impl SysObj for VirtioNode {
    fn id(&self) -> &SysNodeId {
        &self.id
    }

    fn name(&self) -> Cow<'static, str> {
        Cow::Owned(self.name.clone())
    }

    fn type_(&self) -> SysNodeType {
        SysNodeType::Branch
    }

    fn parent(&self) -> Option<alloc::sync::Weak<dyn SysBranchNode>> {
        None
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_node(&self) -> Option<&dyn SysNode> {
        Some(self)
    }

    fn as_branch(&self) -> Option<&dyn SysBranchNode> {
        Some(self)
    }
}

impl SysNode for VirtioNode {
    fn node_attrs(&self) -> &SysAttrSet {
        &self.attrs
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(systree::Error)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<()> {
        Err(systree::Error)
    }
}

impl SysBranchNode for VirtioNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&dyn SysNode>)) {
        let children = self.children.read();
        let child_opt = children.get(name);
        let node_opt = child_opt.and_then(|arc| arc.as_node());
        f(node_opt);
    }

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&dyn SysObj) -> Option<()>) {
        let children = self.children.read();
        for child in children.values() {
            if f(child.as_ref()).is_none() {
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

/// Register virtio node to systree under /devices
pub fn register_virtio_node() {
    let virtio_node = VirtioNode::new();

    let root = systree::singleton().root();

    if let Some(devices_node) = root.child("bus") {
        if let Some(root_node) = devices_node.as_any().downcast_ref::<RootNode>() {
            let _ = root_node.add_child(virtio_node.clone());
        }
    }
}
