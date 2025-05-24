// SPDX-License-Identifier: MPL-2.0

//! Utility definitions and helper structs for implementing `SysTree` nodes.

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::ops::Deref;

use ostd::sync::RwLock;

use super::{
    attr::SysAttrSet,
    node::{SysNodeId, SysObj},
    Error, Result, SysStr,
};
use crate::SysNode;

#[derive(Debug)]
pub struct SysObjFields {
    id: SysNodeId,
    name: SysStr,
}

impl SysObjFields {
    pub fn new(name: SysStr) -> Self {
        Self {
            id: SysNodeId::new(),
            name,
        }
    }

    pub fn id(&self) -> &SysNodeId {
        &self.id
    }

    pub fn name(&self) -> &str {
        self.name.deref()
    }
}

#[derive(Debug)]
pub struct SysNormalNodeFields {
    base: SysObjFields,
    attr_set: SysAttrSet,
}

impl SysNormalNodeFields {
    pub fn new(name: SysStr, attr_set: SysAttrSet) -> Self {
        Self {
            base: SysObjFields::new(name),
            attr_set,
        }
    }

    pub fn id(&self) -> &SysNodeId {
        self.base.id()
    }

    pub fn name(&self) -> &str {
        self.base.name()
    }

    pub fn attr_set(&self) -> &SysAttrSet {
        &self.attr_set
    }
}

#[derive(Debug)]
pub struct SysBranchNodeFields {
    base: SysNormalNodeFields,
    pub children: RwLock<BTreeMap<SysStr, Arc<dyn SysObj>>>,
}

impl SysBranchNodeFields {
    pub fn new(name: SysStr, attr_set: SysAttrSet) -> Self {
        Self {
            base: SysNormalNodeFields::new(name, attr_set),
            children: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn id(&self) -> &SysNodeId {
        self.base.id()
    }

    pub fn name(&self) -> &str {
        self.base.name()
    }

    pub fn attr_set(&self) -> &SysAttrSet {
        self.base.attr_set()
    }

    pub fn contains(&self, child_name: &str) -> bool {
        let children = self.children.read();
        children.contains_key(child_name)
    }

    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()> {
        let mut children = self.children.write();
        let name = new_child.name();
        if children.contains_key(name.deref()) {
            return Err(Error::PermissionDenied);
        }
        children.insert(name.clone(), new_child);
        Ok(())
    }

    pub fn remove_child(&self, child_name: &str) -> Option<Arc<dyn SysObj>> {
        let mut children = self.children.write();
        children.remove(child_name)
    }

    pub fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&dyn SysNode>)) {
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

    pub fn visit_children_with(
        &self,
        _min_id: u64,
        f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>,
    ) {
        let children_guard = self.children.read();
        for child_arc in children_guard.values() {
            if f(child_arc).is_none() {
                break;
            }
        }
    }

    pub fn child(&self, name: &str) -> Option<Arc<dyn SysObj>> {
        let children = self.children.read();
        children.get(name).cloned()
    }

    pub fn children(&self) -> Vec<Arc<dyn SysObj>> {
        let children = self.children.read();
        children.values().cloned().collect()
    }
}

#[derive(Debug)]
pub struct SymlinkNodeFields {
    base: SysObjFields,
    target_path: String,
}

impl SymlinkNodeFields {
    pub fn new(name: SysStr, target_path: String) -> Self {
        Self {
            base: SysObjFields::new(name),
            target_path,
        }
    }

    pub fn id(&self) -> &SysNodeId {
        self.base.id()
    }

    pub fn name(&self) -> &str {
        self.base.name()
    }

    pub fn target_path(&self) -> &str {
        &self.target_path
    }
}
