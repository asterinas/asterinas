// SPDX-License-Identifier: MPL-2.0

use alloc::{
    borrow::Cow,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{any::Any, fmt::Debug};

use ostd::{
    mm::{FallibleVmRead, FallibleVmWrite, VmReader, VmWriter},
    prelude::ktest,
};

use super::{
    Error, Result, SysAttrFlags, SysAttrSet, SysAttrSetBuilder, SysBranchNode, SysBranchNodeFields,
    SysNode, SysNodeId, SysNodeType, SysObj, SysStr, SysSymlink, SysTree,
};

#[derive(Debug)]
struct DeviceNode {
    fields: SysBranchNodeFields<dyn SysObj>,
    self_ref: Weak<Self>,
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
            self_ref: weak_self.clone(),
        })
    }
}

impl SysObj for DeviceNode {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn arc_as_node(&self) -> Option<Arc<dyn SysNode>> {
        self.self_ref
            .upgrade()
            .map(|arc_self| arc_self as Arc<dyn SysNode>)
    }

    fn arc_as_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.self_ref
            .upgrade()
            .map(|arc_self| arc_self as Arc<dyn SysBranchNode>)
    }

    fn id(&self) -> &SysNodeId {
        self.fields.id()
    }

    fn type_(&self) -> SysNodeType {
        SysNodeType::Branch
    }

    fn name(&self) -> SysStr {
        self.fields.name().to_string().into()
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

impl SysBranchNode for DeviceNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&dyn SysNode>)) {
        let children = self.fields.children.read();
        children
            .get(name)
            .map(|child| {
                child
                    .arc_as_node()
                    .map(|node| f(Some(node.as_ref())))
                    .unwrap_or_else(|| f(None))
            })
            .unwrap_or_else(|| f(None));
    }

    fn visit_children_with(&self, min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>) {
        let children = self.fields.children.read();
        for child in children
            .values()
            .filter(|child| child.id().as_u64() >= min_id)
        {
            if f(child).is_none() {
                break;
            }
        }
    }

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>> {
        let children = self.fields.children.read();
        children.get(name).cloned()
    }

    fn children(&self) -> Vec<Arc<dyn SysObj>> {
        let children = self.fields.children.read();
        children.values().cloned().collect()
    }

    fn count_children(&self) -> usize {
        self.fields.children.read().len()
    }
}

#[derive(Debug)]
struct SymlinkNode {
    id: SysNodeId,
    name: SysStr,
    target: String,
    self_ref: Weak<Self>,
}

impl SymlinkNode {
    fn new(name: &str, target: &str) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| SymlinkNode {
            id: SysNodeId::new(),
            name: name.to_string().into(),
            target: target.to_string(),
            self_ref: weak_self.clone(),
        })
    }
}

impl SysObj for SymlinkNode {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn arc_as_symlink(&self) -> Option<Arc<dyn SysSymlink>> {
        self.self_ref
            .upgrade()
            .map(|arc_self| arc_self as Arc<dyn SysSymlink>)
    }

    fn id(&self) -> &SysNodeId {
        &self.id
    }

    fn type_(&self) -> SysNodeType {
        SysNodeType::Symlink
    }

    fn name(&self) -> SysStr {
        self.name.clone()
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
    assert_eq!(root.type_(), SysNodeType::Leaf);
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
    let symlink_node = symlink_obj.arc_as_symlink().unwrap();
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
