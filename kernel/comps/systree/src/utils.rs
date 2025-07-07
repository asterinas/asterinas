// SPDX-License-Identifier: MPL-2.0

//! Utility definitions and helper structs for implementing `SysTree` nodes.

use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
};

use ostd::sync::RwLock;
use spin::Once;

use super::{
    attr::SysAttrSet,
    node::{SysNodeId, SysObj},
    Error, Result, SysStr,
};
use crate::{SysBranchNode, SysNode, SysSymlink};

#[derive(Debug)]
pub struct SysObjFields<T: SysObj> {
    id: SysNodeId,
    name: SysStr,
    parent: Once<Weak<dyn SysBranchNode>>,
    weak_self: Weak<T>,
}

impl<T: SysObj> SysObjFields<T> {
    pub fn new(name: SysStr, weak_self: Weak<T>) -> Self {
        Self {
            id: SysNodeId::new(),
            name,
            parent: Once::new(),
            weak_self,
        }
    }

    pub fn id(&self) -> &SysNodeId {
        &self.id
    }

    pub fn name(&self) -> &SysStr {
        &self.name
    }

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.parent.call_once(|| parent);
    }

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.parent
            .get()
            .and_then(|weak_parent| weak_parent.upgrade())
    }

    pub fn weak_self(&self) -> &Weak<T> {
        &self.weak_self
    }
}

#[derive(Debug)]
pub struct SysNormalNodeFields<T: SysNode> {
    base: SysObjFields<T>,
    attr_set: SysAttrSet,
}

impl<T: SysNode> SysNormalNodeFields<T> {
    pub fn new(name: SysStr, attr_set: SysAttrSet, weak_self: Weak<T>) -> Self {
        Self {
            base: SysObjFields::new(name, weak_self),
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

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.base.init_parent(parent);
    }

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.base.parent()
    }

    pub fn weak_self(&self) -> &Weak<T> {
        self.base.weak_self()
    }
}

#[derive(Debug)]
pub struct SysBranchNodeFields<C: SysObj + ?Sized, T: SysBranchNode> {
    base: SysNormalNodeFields<T>,
    pub children: RwLock<BTreeMap<SysStr, Arc<C>>>,
}

impl<C: SysObj + ?Sized, T: SysBranchNode> SysBranchNodeFields<C, T> {
    pub fn new(name: SysStr, attr_set: SysAttrSet, weak_self: Weak<T>) -> Self {
        Self {
            base: SysNormalNodeFields::new(name, attr_set, weak_self),
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

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.base.init_parent(parent);
    }

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.base.parent()
    }

    pub fn contains(&self, child_name: &str) -> bool {
        let children = self.children.read();
        children.contains_key(child_name)
    }

    pub fn weak_self(&self) -> &Weak<T> {
        self.base.weak_self()
    }

    pub fn add_child(&self, new_child: Arc<C>) -> Result<()> {
        let mut children = self.children.write();
        let name = new_child.name();
        if children.contains_key(name) {
            return Err(Error::PermissionDenied);
        }

        new_child.init_parent(self.weak_self().clone());
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
pub struct SymlinkNodeFields<T: SysSymlink> {
    base: SysObjFields<T>,
    target_path: String,
}

impl<T: SysSymlink> SymlinkNodeFields<T> {
    pub fn new(name: SysStr, target_path: String, weak_self: Weak<T>) -> Self {
        Self {
            base: SysObjFields::new(name, weak_self),
            target_path,
        }
    }

    pub fn id(&self) -> &SysNodeId {
        self.base.id()
    }

    pub fn name(&self) -> &SysStr {
        self.base.name()
    }

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.base.init_parent(parent);
    }

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.base.parent()
    }

    pub fn weak_self(&self) -> &Weak<T> {
        self.base.weak_self()
    }

    pub fn target_path(&self) -> &str {
        &self.target_path
    }
}

/// A macro to automatically generate cast-related methods and `type_` method for `SysObj`
/// trait implementation of `SysBranchNode` struct.
///
/// Users should make sure that the struct has a `fields: SysBranchNodeFields<_, Self>` field.
#[macro_export]
macro_rules! impl_cast_methods_for_branch {
    () => {
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }

        fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
            self.fields
                .weak_self()
                .upgrade()
                .map(|arc| arc as Arc<dyn SysNode>)
        }

        fn cast_to_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
            self.fields
                .weak_self()
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
/// Users should make sure that the struct has a `fields: SysNormalNodeFields<Self>` field.
#[macro_export]
macro_rules! impl_cast_methods_for_node {
    () => {
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }

        fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
            self.fields
                .weak_self()
                .upgrade()
                .map(|arc| arc as Arc<dyn SysNode>)
        }

        fn type_(&self) -> SysNodeType {
            SysNodeType::Leaf
        }
    };
}

/// A macro to automatically generate cast-related methods and `type_` method for `SysObj`
/// trait implementation of `SysSymlink` struct.
///
/// Users should make sure that the struct has a `fields: SymlinkNodeFields` field.
#[macro_export]
macro_rules! impl_cast_methods_for_symlink {
    () => {
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }

        fn cast_to_symlink(&self) -> Option<Arc<dyn SysSymlink>> {
            self.fields
                .weak_self()
                .upgrade()
                .map(|arc| arc as Arc<dyn SysSymlink>)
        }

        fn type_(&self) -> SysNodeType {
            SysNodeType::Symlink
        }
    };
}
