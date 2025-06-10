// SPDX-License-Identifier: MPL-2.0

//! Utility definitions and helper structs for implementing `SysTree` nodes.

use alloc::{collections::BTreeMap, string::String, sync::Arc};
use core::ops::Deref;

use bitflags::bitflags;
use ostd::sync::RwLock;
use spin::Once;

use super::{
    attr::SysAttrSet,
    node::{SysNodeId, SysObj},
    Error, Result, SysStr,
};

#[derive(Debug)]
pub struct SysObjFields {
    id: SysNodeId,
    name: SysStr,
    parent_path: Once<SysStr>,
}

impl SysObjFields {
    pub fn new(name: SysStr) -> Self {
        Self {
            id: SysNodeId::new(),
            name,
            parent_path: Once::new(),
        }
    }

    pub fn id(&self) -> &SysNodeId {
        &self.id
    }

    pub fn name(&self) -> &SysStr {
        &self.name
    }

    pub fn set_parent_path(&self, path: SysStr) {
        self.parent_path.call_once(|| path);
    }

    pub fn path(&self) -> SysStr {
        if let Some(parent_path) = self.parent_path.get() {
            return SysStr::from(parent_path.clone().into_owned() + "/" + self.name.deref());
        }

        self.name().clone()
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

    pub fn set_parent_path(&self, path: SysStr) {
        self.base.set_parent_path(path);
    }

    pub fn path(&self) -> SysStr {
        self.base.path()
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

    pub fn set_parent_path(&self, path: SysStr) {
        self.base.set_parent_path(path);
    }

    pub fn path(&self) -> SysStr {
        self.base.path()
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

        new_child.set_parent_path(self.path());
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

    pub fn set_parent_path(&self, path: SysStr) {
        self.base.set_parent_path(path);
    }

    pub fn path(&self) -> SysStr {
        self.base.path()
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

bitflags! {
    /// Mode and permission representation for the nodes and attributes in the `SysTree`.
    ///
    /// This struct is mainly used to provide the initial permissions for nodes and attributes.
    ///
    /// The concepts of "owner"/"group"/"others" mentioned here are not explicitly represented in
    /// systree. They exist primarily to enable finer-grained permission management at
    /// the "view" and "control" parts for users. Users can provide permission modification functionality
    /// through additional abstractions at the upper layers. Correspondingly, it is the users' responsibility
    /// to do the permission verification at the "view" and "control" parts.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct SysMode: u16 {
        /// Read permission for owner
        const S_IRUSR = 0o0400;
        /// Write permission for owner
        const S_IWUSR = 0o0200;
        /// Execute/search permission for owner
        const S_IXUSR = 0o0100;
        /// Read permission for group
        const S_IRGRP = 0o0040;
        /// Write permission for group
        const S_IWGRP = 0o0020;
        /// Execute/search permission for group
        const S_IXGRP = 0o0010;
        /// Read permission for others
        const S_IROTH = 0o0004;
        /// Write permission for others
        const S_IWOTH = 0o0002;
        /// Execute/search permission for others
        const S_IXOTH = 0o0001;
    }
}

impl SysMode {
    /// Default read-only mode for nodes (owner/group/others can read+execute)
    pub const DEFAULT_RO_MODE: Self = Self::from_bits_truncate(0o555);

    /// Default read-write mode for nodes (owner has full, group/others read+execute)
    pub const DEFAULT_RW_MODE: Self = Self::from_bits_truncate(0o755);

    /// Default read-only mode for attributes (owner/group/others can read)
    pub const DEFAULT_RO_ATTR_MODE: Self = Self::from_bits_truncate(0o444);

    /// Default read-write mode for attributes (owner read+write, group/others read)
    pub const DEFAULT_RW_ATTR_MODE: Self = Self::from_bits_truncate(0o644);

    /// Returns whether this mode has a read permission.
    pub fn can_read(&self) -> bool {
        self.intersects(Self::S_IRUSR | Self::S_IRGRP | Self::S_IROTH)
    }

    /// Returns whether this mode has a write permission.
    pub fn can_write(&self) -> bool {
        self.intersects(Self::S_IWUSR | Self::S_IWGRP | Self::S_IWOTH)
    }
}
