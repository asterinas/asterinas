// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};
use core::fmt::Debug;

use aster_systree::{
    impl_cast_methods_for_branch, Error, Result, SysAttrFlags, SysAttrSet, SysAttrSetBuilder,
    SysBranchNode, SysBranchNodeFields, SysNode, SysNodeId, SysNodeType, SysObj, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

/// The unified cgroup node type.
///
/// This kind of node is used in the v2 unified hierarchy as the root of the cgroup tree.
#[derive(Debug)]
pub(super) struct Unified;

/// The normal cgroup node type.
///
/// Except for the root node, all nodes in the cgroup tree are of this type.
#[derive(Debug)]
pub(super) struct Normal;

/// A node in the cgroup tree, which can be either a [`Unified`] or [`Normal`] cgroup node.
#[derive(Debug)]
pub(super) struct CgroupTreeNode<Type: Debug + Send + Sync + 'static> {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
    _marker: core::marker::PhantomData<Type>,
}

#[inherit_methods(from = "self.fields")]
impl<Type: Debug + Send + Sync + 'static> CgroupTreeNode<Type> {
    /// Adds a child node to this `CgroupTreeNode`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

impl CgroupTreeNode<Unified> {
    /// Creates a new `CgroupTreeNode<Unified>`.
    pub(super) fn new() -> Arc<Self> {
        let name = SysStr::from("cgroup");

        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed.
        builder.add(SysStr::from("cgroup.controllers"), SysAttrFlags::CAN_READ);
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysAttrFlags::CAN_READ | SysAttrFlags::CAN_WRITE,
        );
        builder.add(
            SysStr::from("cgroup.threads"),
            SysAttrFlags::CAN_READ | SysAttrFlags::CAN_WRITE,
        );
        builder.add(
            SysStr::from("cpu.pressure"),
            SysAttrFlags::CAN_READ | SysAttrFlags::CAN_WRITE,
        );
        builder.add(SysStr::from("cpu.stat"), SysAttrFlags::CAN_READ);

        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysBranchNodeFields::new(name, attrs);
        Arc::new_cyclic(|weak_self| CgroupTreeNode::<Unified> {
            fields,
            weak_self: weak_self.clone(),
            _marker: core::marker::PhantomData,
        })
    }
}

impl CgroupTreeNode<Normal> {
    pub(super) fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed. The normal cgroup node may have
        // more attributes than the unified one.
        builder.add(SysStr::from("cgroup.controllers"), SysAttrFlags::CAN_READ);
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysAttrFlags::CAN_READ | SysAttrFlags::CAN_WRITE,
        );
        builder.add(
            SysStr::from("cgroup.threads"),
            SysAttrFlags::CAN_READ | SysAttrFlags::CAN_WRITE,
        );
        builder.add(
            SysStr::from("cpu.pressure"),
            SysAttrFlags::CAN_READ | SysAttrFlags::CAN_WRITE,
        );
        builder.add(SysStr::from("cpu.stat"), SysAttrFlags::CAN_READ);

        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysBranchNodeFields::new(name, attrs);
        Arc::new_cyclic(|weak_self| CgroupTreeNode::<Normal> {
            fields,
            weak_self: weak_self.clone(),
            _marker: core::marker::PhantomData,
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl<Type: Debug + Send + Sync + 'static> SysObj for CgroupTreeNode<Type> {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    default fn is_root(&self) -> bool {
        false
    }

    fn set_path(&self, path: SysStr);

    fn path(&self) -> SysStr;
}

impl SysObj for CgroupTreeNode<Unified> {
    fn is_root(&self) -> bool {
        true
    }
}

impl<Type: Debug + Send + Sync + 'static> SysNode for CgroupTreeNode<Type> {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    default fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        // TODO: Add support for reading attributes.
        Err(Error::AttributeError)
    }

    default fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        // TODO: Add support for writing attributes.
        Err(Error::AttributeError)
    }
}

#[inherit_methods(from = "self.fields")]
impl<Type: Debug + Send + Sync + 'static> SysBranchNode for CgroupTreeNode<Type> {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;
}
