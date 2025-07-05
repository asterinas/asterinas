// SPDX-License-Identifier: MPL-2.0

use alloc::{
    string::ToString,
    sync::{Arc, Weak},
};
use core::fmt::Debug;

use aster_systree::{
    impl_cast_methods_for_branch, Error, Result, SysAttrSet, SysAttrSetBuilder, SysBranchNode,
    SysBranchNodeFields, SysNode, SysNodeId, SysNodeType, SysObj, SysPerms, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

use crate::fs::manager::FsFactory;

/// Represents the root of a cgroup hierarchy, serving as the entry point to
/// the entire cgroup control system.
///
/// The cgroup system provides v2 unified hierarchy, and is also used as a root
/// node in the cgroup systree.
#[derive(Debug)]
pub struct CgroupSystem {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
}

/// A control group node in the cgroup systree.
///
/// Each node can bind a group of processes together for purpose of resource
/// management. Except for the root node, all nodes in the cgroup tree are of
/// this type.
#[derive(Debug)]
pub struct CgroupNode {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
}

impl CgroupSystem {
    /// Adds a child node to this `CgroupSystem`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()> {
        let name = new_child.name();
        let mut children_guard = self.fields.children.write();
        if children_guard.contains_key(name) {
            return Err(Error::AlreadyExists);
        }

        new_child.init_parent_path(SysStr::from(""));
        children_guard.insert(name.clone(), new_child);
        Ok(())
    }
}

#[inherit_methods(from = "self.fields")]
impl CgroupNode {
    /// Adds a child node to this `CgroupNode`.
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

impl CgroupSystem {
    pub(super) fn new() -> Arc<Self> {
        let name = SysStr::from("cgroup");

        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed.
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.threads"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cpu.pressure"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(SysStr::from("cpu.stat"), SysPerms::DEFAULT_RO_ATTR_PERMS);

        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysBranchNodeFields::new(name, attrs);
        Arc::new_cyclic(|weak_self| CgroupSystem {
            fields,
            weak_self: weak_self.clone(),
        })
    }
}

impl CgroupNode {
    pub(super) fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        // TODO: Add more attributes as needed. The normal cgroup node may have
        // more attributes than the unified one.
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.threads"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cpu.pressure"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(SysStr::from("cpu.stat"), SysPerms::DEFAULT_RO_ATTR_PERMS);

        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysBranchNodeFields::new(name, attrs);
        Arc::new_cyclic(|weak_self| CgroupNode {
            fields,
            weak_self: weak_self.clone(),
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for CgroupSystem {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn is_root(&self) -> bool {
        true
    }

    fn init_parent_path(&self, path: SysStr) {
        // This method should be a no-op for `CgroupSystem`.
    }

    fn parent_path(&self) -> Option<&SysStr>;

    fn path(&self) -> SysStr {
        SysStr::from("/")
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for CgroupNode {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn init_parent_path(&self, path: SysStr);

    fn parent_path(&self) -> Option<&SysStr>;
}

impl SysNode for CgroupSystem {
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

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
}

impl SysNode for CgroupNode {
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

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for CgroupSystem {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let new_child = CgroupNode::new(name.to_string().into());
        self.add_child(new_child.clone())?;
        Ok(new_child)
    }

    fn remove_child(&self, name: &str) -> Result<Arc<dyn SysObj>>;
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for CgroupNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let new_child = CgroupNode::new(name.to_string().into());
        self.add_child(new_child.clone())?;
        Ok(new_child)
    }

    fn remove_child(&self, name: &str) -> Result<Arc<dyn SysObj>>;
}

impl FsFactory for CgroupSystem {
    fn on_mount(&self) {
        // When a `CgroupSystem` is registered to `FsManager`, no additional operations
        // are required.
    }

    fn on_unmount(&self) {
        // When a `CgroupSystem` is unregistered from `FsManager`, no additional operations
        // are required.
    }
}
