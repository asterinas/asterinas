// SPDX-License-Identifier: MPL-2.0

use alloc::{
    borrow::Cow,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::fmt::Debug;

use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{FallibleVmRead, FallibleVmWrite, VmReader, VmWriter},
    prelude::ktest,
};

use super::{
    impl_cast_methods_for_branch, impl_cast_methods_for_symlink, Error, Result, SysAttrFlags,
    SysAttrSet, SysAttrSetBuilder, SysBranchNode, SysBranchNodeFields, SysNode, SysNodeId,
    SysNodeType, SysObj, SysStr, SysSymlink, SysTree,
};

#[derive(Debug)]
struct DeviceNode {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
}

impl DeviceNode {
    fn new(name: &str) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        builder
            .add(Cow::Borrowed("model"), SysAttrFlags::CAN_READ)
            .add(Cow::Borrowed("vendor"), SysAttrFlags::CAN_READ)
            .add(
                Cow::Borrowed("status"),
                SysAttrFlags::CAN_READ | SysAttrFlags::CAN_WRITE,
            );

        let attrs = builder.build().expect("Failed to build attribute set");
        let name_owned: SysStr = name.to_string().into();
        let fields = SysBranchNodeFields::new(name_owned, attrs);

        Arc::new_cyclic(|weak_self| DeviceNode {
            fields,
            weak_self: weak_self.clone(),
        })
    }
}

impl SysObj for DeviceNode {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId {
        self.fields.id()
    }

    fn name(&self) -> &SysStr {
        self.fields.name()
    }
}

impl SysNode for DeviceNode {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        // Check if attribute exists
        if !self.fields.attr_set().contains(name) {
            return Err(Error::AttributeError);
        }

        let attr = self.fields.attr_set().get(name).unwrap();
        // Check if attribute is readable
        if !attr.flags().contains(SysAttrFlags::CAN_READ) {
            return Err(Error::PermissionDenied);
        }
        let value = match name {
            "model" => "MyDevice",
            "vendor" => "ExampleVendor",
            "status" => "online",
            _ => "",
        };

        // Write the value to the provided writer
        writer
            .write_fallible(&mut (value.as_bytes()).into())
            .map_err(|_| Error::AttributeError)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        // Get attribute and check if it exists
        let attr = self
            .fields
            .attr_set()
            .get(name)
            .ok_or(Error::AttributeError)?;

        // Check if attribute is writable
        if !attr.flags().contains(SysAttrFlags::CAN_WRITE) {
            return Err(Error::PermissionDenied);
        }

        // Read new value from the provided reader
        let mut buffer = [0u8; 256];
        let mut writer = VmWriter::from(&mut buffer[..]);
        let read_len = reader
            .read_fallible(&mut writer)
            .map_err(|_| Error::AttributeError)?;

        Ok(read_len)
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for DeviceNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;
}

#[derive(Debug)]
struct SymlinkNode {
    id: SysNodeId,
    name: SysStr,
    target: String,
    weak_self: Weak<Self>,
}

impl SymlinkNode {
    fn new(name: &str, target: &str) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| SymlinkNode {
            id: SysNodeId::new(),
            name: name.to_string().into(),
            target: target.to_string(),
            weak_self: weak_self.clone(),
        })
    }
}

impl SysObj for SymlinkNode {
    impl_cast_methods_for_symlink!();

    fn id(&self) -> &SysNodeId {
        &self.id
    }

    fn name(&self) -> &SysStr {
        &self.name
    }
}

impl SysSymlink for SymlinkNode {
    fn target_path(&self) -> &str {
        &self.target
    }
}

#[ktest]
fn systree_singleton() {
    // Get the SysTree singleton
    let sys_tree = SysTree::new();

    // Access the root node
    let root = sys_tree.root();

    // Check if root node exists
    assert!(root.is_root());
    assert_eq!(root.name(), "");
    assert_eq!(root.type_(), SysNodeType::Branch);
}

#[ktest]
fn node_hierarchy() {
    // Create device node hierarchy
    let root_device = DeviceNode::new("root_device");

    // Add child nodes
    {
        let child1 = DeviceNode::new("child1");
        let child2 = DeviceNode::new("child2");
        root_device
            .fields
            .children
            .write()
            .insert(Cow::Borrowed("child1"), child1);
        root_device
            .fields
            .children
            .write()
            .insert(Cow::Borrowed("child2"), child2);
    }

    // Verify number of child nodes
    assert_eq!(root_device.fields.children.read().len(), 2);

    // Get specific child node
    let child = root_device.child("child1").unwrap();
    assert_eq!(child.name(), "child1");

    // Traverse all child nodes
    let all_children: Vec<_> = root_device.children();
    assert_eq!(all_children.len(), 2);
}

#[ktest]
fn attributes() {
    let device = DeviceNode::new("test_device");

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
    let device = DeviceNode::new("device");

    // Create symlink pointing to device
    let symlink = SymlinkNode::new("device_link", "/sys/devices/device");

    // Verify symlink attributes
    assert_eq!(symlink.type_(), SysNodeType::Symlink);
    assert_eq!(symlink.target_path(), "/sys/devices/device");

    // Add symlink to device tree
    {
        device
            .fields
            .children
            .write()
            .insert(Cow::Borrowed("device_link"), symlink.clone());
    }

    // Verify symlink was added correctly
    let symlink_obj = device.child("device_link").unwrap();
    let symlink_node = symlink_obj.cast_to_symlink().unwrap();
    assert_eq!(symlink_node.target_path(), "/sys/devices/device");
}

#[ktest]
fn error_handling() {
    let device = DeviceNode::new("error_test");

    // Attempt to access non-existent attribute
    let result = device.show_attr("nonexistent");
    match result {
        Err(Error::AttributeError) => (),
        _ => panic!("Failed to handle non-existent attribute error"),
    }

    // Attempt to access non-existent child node
    let child = device.child("nonexistent");
    assert!(child.is_none());
}
