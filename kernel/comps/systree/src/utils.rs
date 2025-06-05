// SPDX-License-Identifier: MPL-2.0

//! Utility definitions and helper structs for implementing `SysTree` nodes.

use alloc::{collections::BTreeMap, string::String, sync::Arc};

use ostd::sync::RwLock;

use super::{
    attr::SysAttrSet,
    node::{SysNodeId, SysObj},
    Error, Result, SysStr,
};

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

    pub fn name(&self) -> &SysStr {
        &self.name
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

    pub fn name(&self) -> &SysStr {
        self.base.name()
    }

    pub fn attr_set(&self) -> &SysAttrSet {
        &self.attr_set
    }
}

#[derive(Debug)]
pub struct SysBranchNodeFields<C: SysObj + ?Sized> {
    base: SysNormalNodeFields,
    pub children: RwLock<BTreeMap<SysStr, Arc<C>>>,
}

impl<C: SysObj + ?Sized> SysBranchNodeFields<C> {
    pub fn new(name: SysStr, attr_set: SysAttrSet) -> Self {
        Self {
            base: SysNormalNodeFields::new(name, attr_set),
            children: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn id(&self) -> &SysNodeId {
        self.base.id()
    }

    pub fn name(&self) -> &SysStr {
        self.base.name()
    }

    pub fn attr_set(&self) -> &SysAttrSet {
        self.base.attr_set()
    }

    pub fn contains(&self, child_name: &str) -> bool {
        let children = self.children.read();
        children.contains_key(child_name)
    }

    pub fn add_child(&self, new_child: Arc<C>) -> Result<()> {
        let mut children = self.children.write();
        let name = new_child.name();
        if children.contains_key(name) {
            return Err(Error::PermissionDenied);
        }
        children.insert(name.clone(), new_child);
        Ok(())
    }

    pub fn remove_child(&self, child_name: &str) -> Option<Arc<C>> {
        let mut children = self.children.write();
        children.remove(child_name)
    }

    pub fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<C>>)) {
        let children_guard = self.children.read();
        f(children_guard.get(name))
    }

    pub fn visit_children_with(&self, min_id: u64, f: &mut dyn FnMut(&Arc<C>) -> Option<()>) {
        let children_guard = self.children.read();
        for child_arc in children_guard.values() {
            if child_arc.id().as_u64() < min_id {
                continue;
            }

            if f(child_arc).is_none() {
                break;
            }
        }
    }

    pub fn child(&self, name: &str) -> Option<Arc<C>> {
        let children = self.children.read();
        children.get(name).cloned()
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

    pub fn name(&self) -> &SysStr {
        self.base.name()
    }

    pub fn target_path(&self) -> &str {
        &self.target_path
    }
}

/// A macro to automatically generate cast-related methods and `type_` method for `SysObj`
/// trait implementation of `SysBranchNode` struct.
///
/// Users should make sure that the struct has a `weak_self: Weak<Self>` field.
#[macro_export]
macro_rules! impl_cast_methods_for_branch {
    () => {
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }

        fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
            self.weak_self.upgrade().map(|arc| arc as Arc<dyn SysNode>)
        }

        fn cast_to_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
            self.weak_self
                .upgrade()
                .map(|arc| arc as Arc<dyn SysBranchNode>)
        }

        fn type_(&self) -> SysNodeType {
            SysNodeType::Branch
        }
    };
}

/// A macro to automatically generate cast-related methods and `type_` method for `SysObj`
/// trait implementation of `SysNode` struct.
///
/// If the struct is also a branch node, use `impl_cast_methods_for_branch!()` instead.
///
/// Users should make sure that the struct has a `weak_self: Weak<Self>` field.
#[macro_export]
macro_rules! impl_cast_methods_for_node {
    () => {
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }

        fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
            self.weak_self.upgrade().map(|arc| arc as Arc<dyn SysNode>)
        }

        fn type_(&self) -> SysNodeType {
            SysNodeType::Leaf
        }
    };
}

/// A macro to automatically generate cast-related methods and `type_` method for `SysObj`
/// trait implementation of `SysSymlink` struct.
///
/// Users should make sure that the struct has a `weak_self: Weak<Self>` field.
#[macro_export]
macro_rules! impl_cast_methods_for_symlink {
    () => {
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }

        fn cast_to_symlink(&self) -> Option<Arc<dyn SysSymlink>> {
            self.weak_self
                .upgrade()
                .map(|arc| arc as Arc<dyn SysSymlink>)
        }

        fn type_(&self) -> SysNodeType {
            SysNodeType::Symlink
        }
    };
}
