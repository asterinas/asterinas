// SPDX-License-Identifier: MPL-2.0

//! Utility definitions and helper structs for implementing `SysTree` nodes.

use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec,
};

use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{FallibleVmWrite, VmReader, VmWriter},
    sync::RwLock,
};
use spin::Once;

use super::{
    attr::SysAttrSet,
    node::{SysNodeId, SysObj},
    Error, Result, SysStr,
};
use crate::{SysBranchNode, SysNode, SysSymlink};

/// Fields for all `SysObj` types, including `SysNode` and `SysBranchNode`.
#[derive(Debug)]
pub struct ObjFields<T: SysObj> {
    id: SysNodeId,
    name: SysStr,
    parent: Once<Weak<dyn SysBranchNode>>,
    weak_self: Weak<T>,
}

impl<T: SysObj> ObjFields<T> {
    pub fn new(name: SysStr, weak_self: Weak<T>) -> Self {
        Self {
            id: SysNodeId::new(),
            name,
            parent: Once::new(),
            weak_self,
        }
    }

    pub fn obj_field(&self) -> &ObjFields<T> {
        self
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

/// Fields for normal nodes in the `SysTree`.
#[derive(Debug)]
pub struct NormalNodeFields<T: SysNode> {
    base: ObjFields<T>,
    attr_set: SysAttrSet,
}

#[inherit_methods(from = "self.base")]
impl<T: SysNode> NormalNodeFields<T> {
    pub fn new(name: SysStr, attr_set: SysAttrSet, weak_self: Weak<T>) -> Self {
        Self {
            base: ObjFields::new(name, weak_self),
            attr_set,
        }
    }

    pub fn obj_field(&self) -> &ObjFields<T>;

    pub fn id(&self) -> &SysNodeId;

    pub fn name(&self) -> &SysStr;

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;

    pub fn weak_self(&self) -> &Weak<T>;

    pub fn attr_set(&self) -> &SysAttrSet {
        &self.attr_set
    }
}

/// Fields for attribute-less branch nodes in the `SysTree`.
///
/// An attribute-less branch node is the `SysTree` branch node without any
/// specific attributes.
#[derive(Debug)]
pub struct AttrLessBranchNodeFields<C: SysObj + ?Sized, T: SysBranchNode> {
    base: ObjFields<T>,
    pub children: RwLock<BTreeMap<SysStr, Arc<C>>>,
}

#[inherit_methods(from = "self.base")]
impl<C: SysObj + ?Sized, T: SysBranchNode> AttrLessBranchNodeFields<C, T> {
    pub fn new(name: SysStr, weak_self: Weak<T>) -> Self {
        Self {
            base: ObjFields::new(name, weak_self),
            children: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn obj_field(&self) -> &ObjFields<T>;

    pub fn id(&self) -> &SysNodeId;

    pub fn name(&self) -> &SysStr;

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;

    pub fn weak_self(&self) -> &Weak<T>;

    pub fn contains(&self, child_name: &str) -> bool {
        let children = self.children.read();
        children.contains_key(child_name)
    }

    pub fn add_child(&self, new_child: Arc<C>) -> Result<()> {
        let mut children = self.children.write();
        let name = new_child.name();
        if children.contains_key(name) {
            return Err(Error::AlreadyExists);
        }

        new_child.init_parent(self.weak_self().clone());
        children.insert(name.clone(), new_child);

        Ok(())
    }

    pub fn remove_child(&self, child_name: &str) -> Result<Arc<C>> {
        let mut children = self.children.write();
        children.remove(child_name).ok_or(Error::NotFound)
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

    pub fn children_ref(&self) -> &RwLock<BTreeMap<SysStr, Arc<C>>> {
        &self.children
    }

    pub fn attr_set(&self) -> &SysAttrSet {
        static EMPTY: SysAttrSet = SysAttrSet::new_empty();
        &EMPTY
    }
}

/// Fields for normal branch nodes in the `SysTree`.
#[derive(Debug)]
pub struct BranchNodeFields<C: SysObj + ?Sized, T: SysBranchNode> {
    base: AttrLessBranchNodeFields<C, T>,
    attr_set: SysAttrSet,
}

#[inherit_methods(from = "self.base")]
impl<C: SysObj + ?Sized, T: SysBranchNode> BranchNodeFields<C, T> {
    pub fn new(name: SysStr, attr_set: SysAttrSet, weak_self: Weak<T>) -> Self {
        Self {
            base: AttrLessBranchNodeFields::new(name, weak_self),
            attr_set,
        }
    }

    pub fn obj_field(&self) -> &ObjFields<T>;

    pub fn id(&self) -> &SysNodeId;

    pub fn name(&self) -> &SysStr;

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;

    pub fn weak_self(&self) -> &Weak<T>;

    pub fn contains(&self, child_name: &str) -> bool;

    pub fn add_child(&self, new_child: Arc<C>) -> Result<()>;

    pub fn remove_child(&self, child_name: &str) -> Result<Arc<C>>;

    pub fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<C>>));

    pub fn visit_children_with(&self, min_id: u64, f: &mut dyn FnMut(&Arc<C>) -> Option<()>);

    pub fn child(&self, name: &str) -> Option<Arc<C>>;

    pub fn children_ref(&self) -> &RwLock<BTreeMap<SysStr, Arc<C>>>;

    pub fn attr_set(&self) -> &SysAttrSet {
        &self.attr_set
    }
}

/// Fields for symlink nodes in the `SysTree`.
#[derive(Debug)]
pub struct SymlinkNodeFields<T: SysSymlink> {
    base: ObjFields<T>,
    target_path: String,
}

#[inherit_methods(from = "self.base")]
impl<T: SysSymlink> SymlinkNodeFields<T> {
    pub fn new(name: SysStr, target_path: String, weak_self: Weak<T>) -> Self {
        Self {
            base: ObjFields::new(name, weak_self),
            target_path,
        }
    }

    pub fn obj_field(&self) -> &ObjFields<T>;

    pub fn id(&self) -> &SysNodeId;

    pub fn name(&self) -> &SysStr;

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;

    pub fn weak_self(&self) -> &Weak<T>;

    pub fn target_path(&self) -> &str {
        &self.target_path
    }
}

macro_rules! impl_default_read_attr_at {
    () => {
        fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
            let (attr_buffer, attr_len) = {
                let attr_buffer_len = writer.avail().checked_add(offset).ok_or(Error::Overflow)?;
                let mut buffer = vec![0; attr_buffer_len];
                let len = self.read_attr(
                    name,
                    &mut VmWriter::from(buffer.as_mut_slice()).to_fallible(),
                )?;
                (buffer, len)
            };

            if attr_len <= offset {
                return Ok(0);
            }

            writer
                .write_fallible(VmReader::from(attr_buffer.as_slice()).skip(offset))
                .map_err(|_| Error::AttributeError)
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! _inner_impl_sys_node {
    ($struct_name:ident, $field:ident, $helper_trait:ty) => {
        impl $crate::SysNode for $struct_name {
            fn node_attrs(&self) -> &$crate::SysAttrSet {
                self.$field.attr_set()
            }

            fn read_attr(
                &self,
                name: &str,
                writer: &mut ostd::mm::VmWriter,
            ) -> $crate::Result<usize> {
                <_ as $helper_trait>::read_attr(self, name, writer)
            }

            fn write_attr(
                &self,
                name: &str,
                reader: &mut ostd::mm::VmReader,
            ) -> $crate::Result<usize> {
                <_ as $helper_trait>::write_attr(self, name, reader)
            }

            fn read_attr_at(
                &self,
                name: &str,
                offset: usize,
                writer: &mut ostd::mm::VmWriter,
            ) -> $crate::Result<usize> {
                <_ as $helper_trait>::read_attr_at(self, name, offset, writer)
            }

            fn write_attr_at(
                &self,
                name: &str,
                offset: usize,
                reader: &mut ostd::mm::VmReader,
            ) -> $crate::Result<usize> {
                <_ as $helper_trait>::write_attr_at(self, name, offset, reader)
            }

            fn perms(&self) -> $crate::SysPerms {
                <_ as $helper_trait>::perms(self)
            }
        }
    };
}

/// A helper trait to inherit common methods for `SysTree` leaf types.
#[doc(hidden)]
pub trait _InheritSysLeafNode<T: SysNode> {
    fn field(&self) -> &ObjFields<T>;

    fn is_root(&self) -> bool {
        false
    }

    fn init_parent(&self, parent: alloc::sync::Weak<dyn SysBranchNode>) {
        self.field().init_parent(parent);
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }

    impl_default_read_attr_at!();

    fn write_attr_at(&self, name: &str, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        // In general, the `offset` for attribute write operations is ignored directly.
        self.write_attr(name, reader)
    }

    fn perms(&self) -> crate::SysPerms;
}

/// A convenience macro for implementing [`SysObj`] and [`SysNode`] trait for a `SysTree` leaf type
/// through inheriting the implementation of the target field.
///
/// Users must ensures the target field is a [`NormalNodeFields`] type.
///
/// This macro takes three parameters. The first two parameters correspond to:
/// 1. The name of the struct that implements the `SysObj` trait,
/// 2. The name of the field to be inherited,
///
/// The third parameter is used to implement the required trait methods or override default
/// implementations. For `SysObj` and `SysNode` traits, some of their methods require users
/// to implement them within this macro, some methods provide default implementations that users
/// can override, and some methods provide default implementations that users cannot override.
/// The breakdown is as follows:
///
/// - Methods that require users to implement:
///   - [`SysNode::perms`]
/// - Methods with default implementations that users can override:
///   - [`SysObj::is_root`]
///   - [`SysObj::init_parent`]
///   - [`SysNode::read_attr`]
///   - [`SysNode::write_attr`]
///   - [`SysNode::read_attr_at`]
///   - [`SysNode::write_attr_at`]
/// - Other methods are provided with default implementations that users cannot override.
///
/// Here, "default implementations" refer to either directly inheriting the implementation of a
/// method with the same name from the target `field` or, in the absence of a method with the same
/// name, the default implementation provided by the trait.
///
/// ## Examples
///
/// ```rust
/// /// // A struct representing a `SysTree` leaf node.
/// struct LeafNode {
///     fields: NormalNodeFields<dyn SysObj, Self>,
///     // ... other fields
/// }
///
/// inherit_sys_leaf_node!(
///   LeafNode,      // The struct name.
///   fields,        // The field to be inherited.
///   {
///      fn perms(&self) -> SysPerms {
///        SysPerms::DEFAULT_RW_PERMS
///      }
///   } // Override the `perms` method.
/// );
/// ```
#[macro_export]
macro_rules! inherit_sys_leaf_node {
    ($struct_name:ident, $field:ident, {$($fn_override:item)*}) => {
        impl $crate::_InheritSysLeafNode<$struct_name> for $struct_name {
            fn field(&self) -> &$crate::ObjFields<$struct_name> {
                &self.$field.obj_field()
            }

            $($fn_override)*
        }

        impl $crate::SysObj for $struct_name {
            fn as_any(&self) -> &dyn core::any::Any {
                self
            }

            fn cast_to_node(&self) -> Option<alloc::sync::Arc<dyn $crate::SysNode>> {
                <_ as $crate::_InheritSysLeafNode<$struct_name>>::field(self)
                    .weak_self()
                    .upgrade()
                    .map(|arc| arc as alloc::sync::Arc<dyn $crate::SysNode>)
            }

            fn type_(&self) -> $crate::SysNodeType {
                $crate::SysNodeType::Leaf
            }

            fn id(&self) -> &$crate::SysNodeId {
                self.$field.id()
            }

            fn name(&self) -> &$crate::SysStr {
                self.$field.name()
            }

            fn is_root(&self) -> bool {
                <_ as $crate::_InheritSysLeafNode<$struct_name>>::is_root(self)
            }

            fn init_parent(&self, parent: alloc::sync::Weak<dyn $crate::SysBranchNode>) {
                <_ as $crate::_InheritSysLeafNode<$struct_name>>::init_parent(self, parent)
            }

            fn parent(&self) -> Option<alloc::sync::Arc<dyn $crate::SysBranchNode>> {
                self.$field.parent()
            }
        }

        $crate::_inner_impl_sys_node!($struct_name, $field, $crate::_InheritSysLeafNode<$struct_name>);
    };
}

/// A helper trait to inherit common methods for `SysTree` branch types.
#[doc(hidden)]
pub trait _InheritSysBranchNode<T: SysBranchNode> {
    fn field(&self) -> &ObjFields<T>;

    fn is_root(&self) -> bool {
        false
    }

    fn init_parent(&self, parent: alloc::sync::Weak<dyn SysBranchNode>) {
        self.field().init_parent(parent);
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }

    impl_default_read_attr_at!();

    fn write_attr_at(&self, name: &str, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        // In general, the `offset` for attribute write operations is ignored directly.
        self.write_attr(name, reader)
    }

    fn perms(&self) -> crate::SysPerms;

    fn create_child(&self, _name: &str) -> Result<Arc<dyn SysObj>> {
        Err(Error::PermissionDenied)
    }
}

/// A convenience macro for implementing [`SysObj`], [`SysNode`] and [`SysBranchNode`] trait
/// for a `SysTree` branch type through inheriting the implementation of the target field.
///
/// Users must ensures the target field is a [`BranchNodeFields`] type or a
/// [`AttrLessBranchNodeFields`] type.
///
/// The parameters and requirements of this macro are the same to those of [`inherit_sys_leaf_node`].
/// Here lists the additional override rules for the [`SysBranchNode`] trait methods:
///
/// - No method is required to be implemented by users.
/// - Methods with default implementations that users can override:
///  - [`SysBranchNode::create_child`]
/// - Other methods are provided with default implementations that users cannot override.
///
/// ## Examples
///
/// ```rust
/// /// // A struct representing a `SysTree` branch node.
/// struct BranchNode {
///     fields: BranchNodeFields<dyn SysObj, Self>,
///     // ... other fields
/// }
///
/// inherit_sys_branch_node!{
///   BranchNode,     // The struct name.
///   fields,         // The field to be inherited.
///   {
///      fn perms(&self) -> SysPerms {
///        SysPerms::DEFAULT_RW_PERMS
///      }
///
///      fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
///        //..implementation
///      }
///   } // Override the `perms` and `create_child` methods.
/// }
/// ```
#[macro_export]
macro_rules! inherit_sys_branch_node {
    ($struct_name:ident, $field:ident, {$($fn_override:item)*}) => {
        impl $crate::_InheritSysBranchNode<$struct_name> for $struct_name {
            fn field(&self) -> &$crate::ObjFields<$struct_name> {
                &self.$field.obj_field()
            }

            $($fn_override)*
        }

        impl $crate::SysObj for $struct_name {
            fn as_any(&self) -> &dyn core::any::Any {
                self
            }

            fn cast_to_node(&self) -> Option<alloc::sync::Arc<dyn $crate::SysNode>> {
                <_ as $crate::_InheritSysBranchNode<$struct_name>>::field(self)
                    .weak_self()
                    .upgrade()
                    .map(|arc| arc as alloc::sync::Arc<dyn $crate::SysNode>)
            }

            fn cast_to_branch(&self) -> Option<alloc::sync::Arc<dyn $crate::SysBranchNode>> {
                <_ as $crate::_InheritSysBranchNode<$struct_name>>::field(self)
                    .weak_self()
                    .upgrade()
                    .map(|arc| arc as alloc::sync::Arc<dyn $crate::SysBranchNode>)
            }

            fn type_(&self) -> $crate::SysNodeType {
                $crate::SysNodeType::Branch
            }

            fn id(&self) -> &$crate::SysNodeId {
                self.$field.id()
            }

            fn name(&self) -> &$crate::SysStr {
                self.$field.name()
            }

            fn is_root(&self) -> bool {
                <_ as $crate::_InheritSysBranchNode<$struct_name>>::is_root(self)
            }

            fn init_parent(&self, parent: alloc::sync::Weak<dyn $crate::SysBranchNode>) {
                <_ as $crate::_InheritSysBranchNode<$struct_name>>::init_parent(self, parent)
            }

            fn parent(&self) -> Option<alloc::sync::Arc<dyn $crate::SysBranchNode>> {
                self.$field.parent()
            }
        }

        $crate::_inner_impl_sys_node!($struct_name, $field, $crate::_InheritSysBranchNode<$struct_name>);

        impl $crate::SysBranchNode for $struct_name {
            fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&alloc::sync::Arc<dyn $crate::SysObj>>)) {
                let children_guard = self.$field.children_ref().read();
                let child = children_guard
                    .get(name)
                    .map(|child| child.clone() as alloc::sync::Arc<dyn $crate::SysObj>);

                f(child.as_ref())
            }

            fn visit_children_with(
                &self,
                min_id: u64,
                f: &mut dyn for<'a> FnMut(&'a alloc::sync::Arc<(dyn $crate::SysObj)>) -> Option<()>,
            ) {
                let children_guard = self.$field.children_ref().read();
                for child_arc in children_guard.values() {
                    if child_arc.id().as_u64() < min_id {
                        continue;
                    }

                    let child = child_arc.clone() as alloc::sync::Arc<dyn $crate::SysObj>;
                    if f(&child).is_none() {
                        break;
                    }
                }
            }

            fn child(&self, name: &str) -> Option<Arc<dyn SysObj>> {
                self.$field
                    .child(name)
                    .map(|child| child as Arc<dyn SysObj>)
            }

            fn create_child(&self, name: &str) -> $crate::Result<alloc::sync::Arc<dyn $crate::SysObj>> {
                <_ as $crate::_InheritSysBranchNode<Self>>::create_child(self, name)
            }

            fn remove_child(&self, name: &str) -> $crate::Result<alloc::sync::Arc<dyn $crate::SysObj>> {
                self.$field
                    .remove_child(name)
                    .map(|child| child as Arc<dyn $crate::SysObj>)
            }
        }
    };
}

/// A helper trait to inherit common methods for `SysTree` symlink types.
#[doc(hidden)]
pub trait _InheritSysSymlinkNode<T: SysSymlink> {
    fn field(&self) -> &ObjFields<T>;

    fn is_root(&self) -> bool {
        false
    }

    fn init_parent(&self, parent: alloc::sync::Weak<dyn SysBranchNode>) {
        self.field().init_parent(parent);
    }
}

/// A convenience macro for implementing [`SysObj`] and [`SysSymlink`] trait for a `SysTree` symlink
/// type through inheriting the implementation of the target field.
///
/// Users must ensures the target field is a [`SymlinkNodeFields`] type.
///
/// The parameters and requirements of this macro are similar to [`inherit_sys_leaf_node`],
/// except that the third parameter is optional. This macro follows the same override rules  
/// as the [`SysObj`] component in `inherit_sys_leaf_node`, allowing users to use it  
/// without implementing or overriding any methods.
///
/// ## Examples
///
/// ```rust
/// /// // A struct representing a `SysTree` symlink node.
/// struct SymlinkNode {
///     fields: SymlinkNodeFields<dyn SysObj, Self>,
///     // ... other fields
/// }
///
/// inherit_sys_symlink_node!(
///   BranchNode,     // The struct name.
///   fields         // The field to be inherited.
/// ); // No overrides.
/// ```
#[macro_export]
macro_rules! inherit_sys_symlink_node {
    ($struct_name:ident, $field:ident, {$($fn_override:item)*}) => {
        impl $crate::_InheritSysSymlinkNode<$struct_name> for $struct_name {
            fn field(&self) -> &$crate::ObjFields<$struct_name> {
                &self.$field.obj_field()
            }

            $($fn_override)*
        }

        impl $crate::SysObj for $struct_name {
            fn as_any(&self) -> &dyn core::any::Any {
                self
            }

            fn cast_to_symlink(&self) -> Option<Arc<dyn $crate::SysSymlink>> {
                <_ as $crate::_InheritSysSymlinkNode<$struct_name>>::field(self)
                    .weak_self()
                    .upgrade()
                    .map(|arc| arc as Arc<dyn $crate::SysSymlink>)
            }

            fn type_(&self) -> $crate::SysNodeType {
                $crate::SysNodeType::Symlink
            }

            fn id(&self) -> &$crate::SysNodeId {
                self.$field.id()
            }

            fn name(&self) -> &$crate::SysStr {
                self.$field.name()
            }

            fn is_root(&self) -> bool {
                <_ as $crate::_InheritSysSymlinkNode<$struct_name>>::is_root(self)
            }

            fn init_parent(&self, parent: alloc::sync::Weak<dyn $crate::SysBranchNode>) {
                <_ as $crate::_InheritSysSymlinkNode<$struct_name>>::init_parent(self, parent)
            }

            fn parent(&self) -> Option<alloc::sync::Arc<dyn $crate::SysBranchNode>> {
                self.$field.parent()
            }
        }

        impl $crate::SysSymlink for $struct_name {
            fn target_path(&self) -> &str {
                self.$field.target_path()
            }
        }
    };

    ($struct_name:ident, $field:ident) => {
        $crate::inherit_sys_symlink_node!($struct_name, $field, {/* no overrides */});
    };
}
