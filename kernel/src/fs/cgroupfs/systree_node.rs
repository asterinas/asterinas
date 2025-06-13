// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};
use core::fmt::Debug;

use aster_systree::{
    impl_cast_methods_for_branch, Error, Result, SysAttrSet, SysAttrSetBuilder, SysBranchNode,
    SysBranchNodeFields, SysMode, SysNode, SysNodeId, SysNodeType, SysObj, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

/// A node in the cgroup systree, which represents the unified cgroup node.
///
/// This kind of node is used in the v2 unified hierarchy as the root of the cgroup tree.
#[derive(Debug)]
pub struct CgroupUnifiedNode {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
}

/// A node in the cgroup systree, which represents a normal cgroup node.
///
/// Except for the root node, all nodes in the cgroup tree are of this type.
#[derive(Debug)]
pub struct CgroupNormalNode {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
}

impl CgroupUnifiedNode {
    /// Adds a child node to this `CgroupUnifiedNode`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()> {
        let name = new_child.name();
        let mut children_guard = self.fields.children.write();
        if children_guard.contains_key(name) {
            return Err(Error::PermissionDenied);
        }

        new_child.set_parent_path(SysStr::from(""));
        children_guard.insert(name.clone(), new_child);
        Ok(())
    }
}

#[inherit_methods(from = "self.fields")]
impl CgroupNormalNode {
    /// Adds a child node to this `CgroupNormalNode`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

impl CgroupUnifiedNode {
    pub(super) fn new() -> Arc<Self> {
        let name = SysStr::from("cgroup");

        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed.
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysMode::DEFAULT_RO_ATTR_MODE,
        );
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysMode::DEFAULT_RW_ATTR_MODE,
        );
        builder.add(
            SysStr::from("cgroup.threads"),
            SysMode::DEFAULT_RW_ATTR_MODE,
        );
        builder.add(SysStr::from("cpu.pressure"), SysMode::DEFAULT_RW_ATTR_MODE);
        builder.add(SysStr::from("cpu.stat"), SysMode::DEFAULT_RO_ATTR_MODE);

        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysBranchNodeFields::new(name, attrs);
        Arc::new_cyclic(|weak_self| CgroupUnifiedNode {
            fields,
            weak_self: weak_self.clone(),
        })
    }
}

impl CgroupNormalNode {
    pub(super) fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed. The normal cgroup node may have
        // more attributes than the unified one.
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysMode::DEFAULT_RO_ATTR_MODE,
        );
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysMode::DEFAULT_RW_ATTR_MODE,
        );
        builder.add(
            SysStr::from("cgroup.threads"),
            SysMode::DEFAULT_RW_ATTR_MODE,
        );
        builder.add(SysStr::from("cpu.pressure"), SysMode::DEFAULT_RW_ATTR_MODE);
        builder.add(SysStr::from("cpu.stat"), SysMode::DEFAULT_RO_ATTR_MODE);

        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysBranchNodeFields::new(name, attrs);
        Arc::new_cyclic(|weak_self| CgroupNormalNode {
            fields,
            weak_self: weak_self.clone(),
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for CgroupUnifiedNode {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn is_root(&self) -> bool {
        true
    }

    fn path(&self) -> SysStr {
        SysStr::from("/")
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for CgroupNormalNode {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn is_root(&self) -> bool {
        false
    }

    fn set_parent_path(&self, path: SysStr);

    fn path(&self) -> SysStr;
}

impl SysNode for CgroupUnifiedNode {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        // TODO: Add support for reading attributes.
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        // TODO: Add support for writing attributes.
        Err(Error::AttributeError)
    }

    fn mode(&self) -> SysMode {
        SysMode::DEFAULT_RW_MODE
    }
}

impl SysNode for CgroupNormalNode {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        // TODO: Add support for reading attributes.
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        // TODO: Add support for writing attributes.
        Err(Error::AttributeError)
    }

    fn mode(&self) -> SysMode {
        SysMode::DEFAULT_RW_MODE
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for CgroupUnifiedNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for CgroupNormalNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;
}
