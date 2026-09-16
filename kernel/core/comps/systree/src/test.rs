// SPDX-License-Identifier: MPL-2.0

use alloc::{borrow::Cow, string::ToString, sync::Arc, vec::Vec};
use core::fmt::Debug;

use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{FallibleVmRead, VmReader, VmWriter},
    prelude::ktest,
};

use super::{
    BranchNodeFields, Error, Result, SymlinkNodeFields, SysAttrSetBuilder, SysBranchNode, SysNode,
    SysNodeType, SysObj, SysPerms, SysStr, SysSymlink, SysTree, inherit_sys_branch_node,
    inherit_sys_symlink_node,
};

#[derive(Debug)]
struct DeviceNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

impl DeviceNode {
    fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        builder
            .add(Cow::Borrowed("model"), SysPerms::DEFAULT_RO_ATTR_PERMS)
            .add(Cow::Borrowed("vendor"), SysPerms::DEFAULT_RO_ATTR_PERMS)
            .add(Cow::Borrowed("status"), SysPerms::DEFAULT_RW_ATTR_PERMS);

        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            DeviceNode { fields }
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl DeviceNode {
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

inherit_sys_branch_node!(DeviceNode, fields, {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        // Check if attribute exists
        if !self.fields.attr_set().contains(name) {
            return Err(Error::NotFound);
        }

        let attr = self.fields.attr_set().get(name).unwrap();
        // Check if attribute is readable
        if !attr.perms().can_read() {
            return Err(Error::PermissionDenied);
        }
        let value = match name {
            "model" => "MyDevice",
            "vendor" => "ExampleVendor",
            "status" => "online",
            _ => "",
        };

        let mut printer = VmPrinter::new_skip(writer, offset);
        // Write the value to the provided writer
        write!(printer, "{}", value)?;

        Ok(printer.bytes_written())
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        // Get attribute and check if it exists
        let attr = self.fields.attr_set().get(name).ok_or(Error::NotFound)?;

        // Check if attribute is writable
        if !attr.perms().can_write() {
            return Err(Error::PermissionDenied);
        }

        // Read new value from the provided reader
        let mut buffer = [0u8; 256];
        let mut writer = VmWriter::from(&mut buffer[..]);
        let read_len = reader
            .read_fallible(&mut writer)
            .map_err(|_| Error::PageFault)?;

        Ok(read_len)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[derive(Debug)]
struct SymlinkNode {
    fields: SymlinkNodeFields<Self>,
}

impl SymlinkNode {
    fn new(name: SysStr, target: &str) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let fields = SymlinkNodeFields::new(name, target.to_string(), weak_self.clone());
            SymlinkNode { fields }
        })
    }
}

inherit_sys_symlink_node!(SymlinkNode, fields);

#[ktest]
fn systree_singleton() {
    // Get the SysTree singleton
    let sys_tree = SysTree::new();

    // Access the root node
    let root = sys_tree.root();

    // Check if root node exists
    assert!(root.is_root());
    assert_eq!(root.name(), "");
    assert_eq!(root.path(), "/");
    assert_eq!(root.type_(), SysNodeType::Branch);
}

#[ktest]
fn node_path() {
    let sys_tree = SysTree::new();
    let root = sys_tree.root();

    assert_eq!(root.path(), "/");

    let device = DeviceNode::new("device".into());
    let sub_device_1 = DeviceNode::new("sub_device_1".into());
    // Add the child node to `device` before attaching it to the `SysTree`.
    device.add_child(sub_device_1.clone()).unwrap();
    root.add_child(device.clone()).unwrap();

    assert_eq!(device.path(), "/device");
    assert_eq!(sub_device_1.path(), "/device/sub_device_1");

    let sub_device_2 = DeviceNode::new("sub_device_2".into());
    // Add the child node to `device` after attaching it to the `SysTree`.
    device.add_child(sub_device_2.clone()).unwrap();
    assert_eq!(sub_device_2.path(), "/device/sub_device_2");
}

#[ktest]
fn node_hierarchy() {
    // Create device node hierarchy
    let root_device = DeviceNode::new("root_device".into());

    // Add child nodes
    {
        let child1 = DeviceNode::new("child1".into());
        let child2 = DeviceNode::new("child2".into());
        root_device.add_child(child1).unwrap();
        root_device.add_child(child2).unwrap();
    }

    // Verify number of child nodes
    assert_eq!(root_device.count_children(), 2);

    // Get specific child node
    let child = root_device.child("child1").unwrap();
    assert_eq!(child.name(), "child1");

    // Traverse all child nodes
    let all_children: Vec<_> = root_device.children();
    assert_eq!(all_children.len(), 2);
}

#[ktest]
fn attributes() {
    let device = DeviceNode::new("test_device".into());

    // Read read-only attribute
    let model = device.show_attr("model").unwrap();
    assert_eq!(model, "MyDevice");

    // Read read-write attribute
    let status = device.show_attr("status").unwrap();
    assert_eq!(status, "online");

    // Modify read-write attribute
    let len = device.store_attr("status", "offline").unwrap();
    assert_eq!(len, 7);

    // Attempt to modify read-only attribute (should fail)
    let result = device.store_attr("model", "NewModel");
    assert!(result.is_err());
}

#[ktest]
fn symlinks() {
    let device = DeviceNode::new("device".into());

    // Create symlink pointing to device
    let symlink = SymlinkNode::new("device_link".into(), "/sys/devices/device");

    // Verify symlink attributes
    assert_eq!(symlink.type_(), SysNodeType::Symlink);
    assert_eq!(symlink.target_path(), "/sys/devices/device");

    // Add symlink to device tree
    {
        device.add_child(symlink.clone()).unwrap();
    }

    // Verify symlink was added correctly
    let symlink_obj = device.child("device_link").unwrap();
    let symlink_node = symlink_obj.cast_to_symlink().unwrap();
    assert_eq!(symlink_node.target_path(), "/sys/devices/device");
}

#[ktest]
fn error_handling() {
    let device = DeviceNode::new("error_test".into());

    // Attempt to access non-existent attribute
    let result = device.show_attr("nonexistent");
    match result {
        Err(Error::NotFound) => (),
        _ => panic!("Failed to handle non-existent attribute error"),
    }

    // Attempt to access non-existent child node
    let child = device.child("nonexistent");
    assert!(child.is_none());
}

#[ktest]
#[should_panic]
fn node_name_rejects_empty_at_construction() {
    DeviceNode::new("".into());
}

#[ktest]
#[should_panic]
fn symlink_name_rejects_empty_at_construction() {
    SymlinkNode::new("".into(), "target");
}

#[ktest]
#[should_panic]
fn node_name_rejects_slash_at_construction() {
    DeviceNode::new("bad/name".into());
}

#[ktest]
#[should_panic]
fn symlink_name_rejects_nul_at_construction() {
    SymlinkNode::new("bad\0name".into(), "target");
}

#[ktest]
#[should_panic]
fn node_name_rejects_dot_at_construction() {
    DeviceNode::new(".".into());
}

#[ktest]
#[should_panic]
fn node_name_rejects_dot_dot_at_construction() {
    DeviceNode::new("..".into());
}

#[ktest]
fn invalid_attribute_names_reject_the_new_set() {
    let mut builder = SysAttrSetBuilder::new();
    builder.add("valid".into(), SysPerms::DEFAULT_RO_ATTR_PERMS);
    let original = builder.build().unwrap();
    for name in ["", ".", "..", "bad/name", "bad\0name"] {
        let mut builder = SysAttrSetBuilder::from_set(&original);
        builder.add(name.into(), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.remove(name);
        assert!(matches!(builder.build(), Err(Error::InvalidName)));
    }
    assert_eq!(original.len(), 1);
    assert!(original.contains("valid"));
}

#[ktest]
fn attribute_capacity_returns_error_without_wrapping_ids() {
    let max_attrs = crate::SysAttrSet::ID_CAPACITY - 1;
    let mut builder = SysAttrSetBuilder::new();
    for index in 0..max_attrs {
        builder.add(
            alloc::format!("attr{index}").into(),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
    }
    let attrs = builder.build().unwrap();
    assert_eq!(attrs.len(), max_attrs);
    assert_eq!(attrs.get("attr0").unwrap().id(), 1);
    let last_index = max_attrs - 1;
    let last_attr = attrs.get(&alloc::format!("attr{last_index}")).unwrap();
    assert_eq!(last_attr.id(), u8::MAX);

    let mut builder = SysAttrSetBuilder::new();
    for index in 0..=max_attrs {
        builder.add(
            alloc::format!("attr{index}").into(),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
    }
    assert!(matches!(builder.build(), Err(Error::ResourceUnavailable)));
}

#[ktest]
fn rebuilding_attributes_preserves_survivors_and_reuses_free_ids() {
    let mut builder = SysAttrSetBuilder::new();
    for name in ["first", "second", "third"] {
        builder.add(name.into(), SysPerms::DEFAULT_RO_ATTR_PERMS);
    }
    let old = builder.build().unwrap();
    let mut builder = SysAttrSetBuilder::from_set(&old);
    builder.remove("second");
    builder.add("fourth".into(), SysPerms::DEFAULT_RW_ATTR_PERMS);
    let new = builder.build().unwrap();
    assert_eq!(
        new.get("first").unwrap().id(),
        old.get("first").unwrap().id()
    );
    assert_eq!(
        new.get("third").unwrap().id(),
        old.get("third").unwrap().id()
    );
    assert_eq!(
        new.get("fourth").unwrap().id(),
        old.get("second").unwrap().id()
    );
    assert!(old.contains("second"));
    assert!(!old.contains("fourth"));
    assert!(!new.contains("second"));
}
