// SPDX-License-Identifier: MPL-2.0

use alloc::{
    borrow::Cow,
    collections::BTreeMap,
    format,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::fmt::Debug;

use aster_systree::{
    inherit_sys_branch_node, inherit_sys_leaf_node, inherit_sys_symlink_node, init_for_ktest,
    singleton as systree_singleton, BranchNodeFields, Error as SysTreeError, NormalNodeFields,
    Result as SysTreeResult, SymlinkNodeFields, SysAttrSetBuilder, SysObj, SysPerms, SysStr,
    SysTree,
};
use ostd::{
    mm::{FallibleVmRead, FallibleVmWrite, VmReader, VmWriter},
    prelude::ktest,
    sync::RwLock,
};

use crate::{
    fs::{
        sysfs::fs::SysFs,
        utils::{DirentVisitor, FileSystem, InodeMode, InodeType},
    },
    time::clocks::init_for_ktest as time_init_for_ktest,
    Result,
};

// --- Mock SysTree Components ---
// Sysfs acts as a view layer over the systree component.
// These mocks simulate the systree interface (SysNode, SysBranchNode, etc.)

// Refactor MockLeafNode to use NormalNodeFields
#[derive(Debug)]
struct MockLeafNode {
    fields: NormalNodeFields<Self>,
    data: RwLock<BTreeMap<String, String>>, // Store attribute data
}

impl MockLeafNode {
    fn new(name: SysStr, read_attrs: &[&str], write_attrs: &[&str]) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        let mut data = BTreeMap::new();
        for &attr_name in read_attrs {
            builder.add(
                Cow::Owned(attr_name.to_string()),
                SysPerms::DEFAULT_RO_ATTR_PERMS,
            );
            data.insert(attr_name.to_string(), format!("val_{}", attr_name)); // Initial value
        }
        for &attr_name in write_attrs {
            builder.add(
                Cow::Owned(attr_name.to_string()),
                SysPerms::DEFAULT_RW_ATTR_PERMS,
            );
            data.insert(attr_name.to_string(), format!("val_{}", attr_name)); // Initial value
        }

        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = NormalNodeFields::new(name, attrs, weak_self.clone());
            MockLeafNode {
                fields,
                data: RwLock::new(data),
            }
        })
    }
}

inherit_sys_leaf_node!(MockLeafNode, fields, {
    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> SysTreeResult<usize> {
        let attr = self
            .fields
            .attr_set()
            .get(name)
            .ok_or(SysTreeError::NotFound)?;
        if !attr.perms().can_read() {
            return Err(SysTreeError::PermissionDenied);
        }
        let data = self.data.read();
        let value = data.get(name).ok_or(SysTreeError::AttributeError)?; // Should exist if in attrs
        let bytes = value.as_bytes();
        writer
            .write_fallible(&mut bytes.into())
            .map_err(|_| SysTreeError::AttributeError)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> SysTreeResult<usize> {
        let attr = self
            .fields
            .attr_set()
            .get(name)
            .ok_or(SysTreeError::NotFound)?;
        if !attr.perms().can_write() {
            return Err(SysTreeError::PermissionDenied);
        }

        let mut buffer = [0u8; 1024]; // Max write size for test
        let mut writer = VmWriter::from(&mut buffer[..]);
        let read_len = reader
            .read_fallible(&mut writer)
            .map_err(|_| SysTreeError::AttributeError)?;

        let new_value = String::from_utf8_lossy(&buffer[..read_len]).to_string();

        let mut data = self.data.write();
        data.insert(name.to_string(), new_value);

        Ok(read_len)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

// Refactor MockBranchNode to use BranchNodeFields
#[derive(Debug)]
struct MockBranchNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

impl MockBranchNode {
    fn new(name: &str) -> Arc<Self> {
        let name_owned: SysStr = name.to_string().into(); // Convert to owned SysStr

        let mut builder = SysAttrSetBuilder::new();
        builder.add(
            Cow::Borrowed("branch_attr"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        let attrs = builder
            .build()
            .expect("Failed to build branch attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name_owned, attrs, weak_self.clone());
            MockBranchNode { fields }
        })
    }

    fn add_child(&self, child: Arc<dyn SysObj>) {
        self.fields.add_child(child).unwrap();
    }
}

inherit_sys_branch_node!(MockBranchNode, fields, {
    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> SysTreeResult<usize> {
        let attr = self
            .fields
            .attr_set()
            .get(name)
            .ok_or(SysTreeError::NotFound)?;
        if !attr.perms().can_read() {
            return Err(SysTreeError::PermissionDenied);
        }
        let value = match name {
            "branch_attr" => "branch_value",
            _ => return Err(SysTreeError::AttributeError),
        };
        let bytes = value.as_bytes();
        writer
            .write_fallible(&mut bytes.into())
            .map_err(|_| SysTreeError::AttributeError)
    }

    fn write_attr(&self, name: &str, _reader: &mut VmReader) -> SysTreeResult<usize> {
        let attr = self
            .fields
            .attr_set()
            .get(name)
            .ok_or(SysTreeError::NotFound)?;
        if !attr.perms().can_write() {
            return Err(SysTreeError::PermissionDenied);
        }
        // No writable attrs in this mock for now
        Err(SysTreeError::AttributeError)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

// Mock Symlink
#[derive(Debug)]
struct MockSymlinkNode {
    fields: SymlinkNodeFields<Self>,
}

impl MockSymlinkNode {
    fn new(name: SysStr, target: &str) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let fields = SymlinkNodeFields::new(name, target.to_string(), weak_self.clone());
            MockSymlinkNode { fields }
        })
    }
}

inherit_sys_symlink_node!(MockSymlinkNode, fields);

// --- Test Setup ---

// Create a mock SysTree instance populated with mock nodes.
fn create_mock_systree_instance() -> &'static Arc<SysTree> {
    time_init_for_ktest();
    init_for_ktest();
    // Create nodes
    let root = systree_singleton().root();
    let branch1 = MockBranchNode::new("branch1");
    let leaf1 = MockLeafNode::new("leaf1".into(), &["r_attr1"], &["rw_attr1"]);
    let leaf2 = MockLeafNode::new("leaf2".into(), &["r_attr2"], &[]);
    let symlink1 = MockSymlinkNode::new("link1".into(), "../branch1/leaf1");

    // Build hierarchy - ignore Result since this is test setup
    branch1.add_child(leaf1.clone() as Arc<dyn SysObj>);
    let _ = root.add_child(branch1.clone() as Arc<dyn SysObj>);
    let _ = root.add_child(leaf2.clone() as Arc<dyn SysObj>);
    let _ = root.add_child(symlink1.clone() as Arc<dyn SysObj>);

    systree_singleton()
}

// Initialize a SysFs instance using the mock systree.
fn init_sysfs_with_mock_tree() -> Arc<SysFs> {
    let _ = create_mock_systree_instance();
    SysFs::new()
}

#[ktest]
fn test_sysfs_root_lookup() {
    // Setup: Create SysFs instance backed by the mock systree
    let sysfs = init_sysfs_with_mock_tree();
    let root_inode = sysfs.root_inode(); // Get the sysfs root inode

    // Verification: Check that the sysfs root inode corresponds to the mock systree root

    assert_eq!(root_inode.type_(), InodeType::Dir);

    // Lookup existing branch
    let branch1_inode = root_inode.lookup("branch1").expect("Lookup branch1 failed");
    assert_eq!(branch1_inode.type_(), InodeType::Dir);

    // Lookup existing leaf (represented as Dir in sysfs)
    let leaf2_inode = root_inode.lookup("leaf2").expect("Lookup leaf2 failed");
    assert_eq!(leaf2_inode.type_(), InodeType::Dir);

    // Lookup existing symlink
    let link1_inode = root_inode.lookup("link1").expect("Lookup link1 failed");
    assert_eq!(link1_inode.type_(), InodeType::SymLink);

    // Lookup non-existent
    let result = root_inode.lookup("nonexistent");
    assert!(result.is_err());

    // Lookup "."
    let self_inode = root_inode.lookup(".").expect("Lookup . failed");
    assert_eq!(self_inode.ino(), root_inode.ino());

    // Lookup ".." from root
    let parent_inode = root_inode.lookup("..").expect("Lookup .. failed");
    assert_eq!(parent_inode.ino(), root_inode.ino()); // Parent of root is root
}

#[ktest]
fn test_sysfs_branch_lookup() {
    let sysfs = init_sysfs_with_mock_tree();
    let root_inode = sysfs.root_inode();
    // Action: Lookup a branch node within sysfs
    let branch1_inode = root_inode.lookup("branch1").unwrap();

    // Verification: Check lookups within the sysfs branch inode,
    // ensuring they correctly reflect the children and attributes of the underlying mock systree branch node.

    // Lookup existing leaf inside branch
    let leaf1_inode = branch1_inode.lookup("leaf1").expect("Lookup leaf1 failed");
    assert_eq!(leaf1_inode.type_(), InodeType::Dir); // Leaf nodes are Dirs

    // Lookup branch attribute
    let attr_inode = branch1_inode
        .lookup("branch_attr")
        .expect("Lookup branch_attr failed");
    assert_eq!(attr_inode.type_(), InodeType::File);

    // Lookup non-existent inside branch
    let result = branch1_inode.lookup("nonexistent_leaf");
    assert!(result.is_err());

    // Lookup "."
    let self_inode = branch1_inode.lookup(".").expect("Lookup . failed");
    assert_eq!(self_inode.ino(), branch1_inode.ino());

    // Lookup ".."
    let parent_inode = branch1_inode.lookup("..").expect("Lookup .. failed");
    assert_eq!(parent_inode.ino(), root_inode.ino());
}

#[ktest]
fn test_sysfs_leaf_lookup() {
    let sysfs = init_sysfs_with_mock_tree();
    let root_inode = sysfs.root_inode();
    // Action: Lookup a leaf node (represented as a directory in sysfs)
    let leaf1_inode = root_inode
        .lookup("branch1")
        .unwrap()
        .lookup("leaf1")
        .unwrap();

    // Verification: Check lookups within the sysfs leaf directory,
    // ensuring they correctly reflect the attributes of the underlying mock systree leaf node.

    assert_eq!(leaf1_inode.type_(), InodeType::Dir); // Leaf node itself is Dir

    // Lookup existing readable attribute
    let r_attr_inode = leaf1_inode
        .lookup("r_attr1")
        .expect("Lookup r_attr1 failed");
    assert_eq!(r_attr_inode.type_(), InodeType::File);

    // Lookup existing read-write attribute
    let rw_attr_inode = leaf1_inode
        .lookup("rw_attr1")
        .expect("Lookup rw_attr1 failed");
    assert_eq!(rw_attr_inode.type_(), InodeType::File);

    // Lookup non-existent attribute
    let result = leaf1_inode.lookup("nonexistent_attr");
    assert!(result.is_err());

    // Lookup "."
    let self_inode = leaf1_inode.lookup(".").expect("Lookup . failed");
    assert_eq!(self_inode.ino(), leaf1_inode.ino());
}

#[ktest]
fn test_sysfs_read_attr() {
    let sysfs = init_sysfs_with_mock_tree();
    let root_inode = sysfs.root_inode();
    let leaf1_dir_inode = root_inode
        .lookup("branch1")
        .unwrap()
        .lookup("leaf1")
        .unwrap();
    // Action: Lookup the sysfs file corresponding to a systree attribute
    let r_attr_inode = leaf1_dir_inode.lookup("r_attr1").unwrap();

    // Verification: Read the sysfs file and check if the content matches
    // the data provided by the underlying mock systree node's read_attr method.

    let mut buf = [0u8; 64];
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    let bytes_read = r_attr_inode
        .read_at(0, &mut writer)
        .expect("read_at failed");

    assert!(bytes_read > 0);
    let content = core::str::from_utf8(&buf[..bytes_read]).unwrap();
    assert_eq!(content, "val_r_attr1");

    // Reading a directory should fail (expect EINVAL as per inode.rs)
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible(); // Reset writer
    let result = leaf1_dir_inode.read_at(0, &mut writer);
    assert!(result.is_err());
}

#[ktest]
fn test_sysfs_write_attr() {
    let sysfs = init_sysfs_with_mock_tree();
    let root_inode = sysfs.root_inode();
    let leaf1_dir_inode = root_inode
        .lookup("branch1")
        .unwrap()
        .lookup("leaf1")
        .unwrap();
    // Action: Lookup sysfs files for attributes
    let rw_attr_inode = leaf1_dir_inode.lookup("rw_attr1").unwrap();
    let r_attr_inode = leaf1_dir_inode.lookup("r_attr1").unwrap();

    // Verification: Write to the sysfs files and check if the operation
    // is correctly delegated to the underlying mock systree node's write_attr method,
    // respecting read/write permissions derived from SysPerms.

    // Write to rw_attr1
    let new_val = "new_value";
    let mut reader = VmReader::from(new_val.as_bytes()).to_fallible();
    let bytes_written = rw_attr_inode
        .write_at(0, &mut reader)
        .expect("write_at failed");
    assert_eq!(bytes_written, new_val.len());

    // Read back to verify
    let mut buf = [0u8; 64];
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    let bytes_read = rw_attr_inode
        .read_at(0, &mut writer)
        .expect("read_at failed");
    let content = core::str::from_utf8(&buf[..bytes_read]).unwrap();
    assert_eq!(content, new_val);

    // Write to r_attr1 (should fail - EIO expected from underlying PermissionDenied)
    let mut reader = VmReader::from("attempt_write".as_bytes()).to_fallible();
    let result = r_attr_inode.write_at(0, &mut reader);
    assert!(result.is_err());

    // Writing to a directory should fail (expect EINVAL as per inode.rs)
    let mut reader = VmReader::from("attempt_write".as_bytes()).to_fallible();
    let result = leaf1_dir_inode.write_at(0, &mut reader);
    assert!(result.is_err());
}

#[ktest]
fn test_sysfs_read_link() {
    let sysfs = init_sysfs_with_mock_tree();
    let root_inode = sysfs.root_inode();
    // Action: Lookup the sysfs symlink corresponding to a systree symlink node
    let link1_inode = root_inode.lookup("link1").unwrap();

    // Verification: Read the sysfs symlink and check if the target path matches
    // the path provided by the underlying mock systree symlink node's target_path method.

    let target = link1_inode.read_link().expect("read_link failed");
    assert_eq!(target, "../branch1/leaf1");

    // read_link on non-symlink should fail (expect EINVAL as per inode.rs)
    let branch1_inode = root_inode.lookup("branch1").unwrap();
    let result = branch1_inode.read_link();
    assert!(result.is_err());
}

// Helper for readdir tests
struct TestDirentVisitor {
    entries: Vec<(String, u64, InodeType)>,
}

impl DirentVisitor for TestDirentVisitor {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, _next_offset: usize) -> Result<()> {
        self.entries.push((name.to_string(), ino, type_));
        Ok(())
    }
}

#[ktest]
fn test_sysfs_readdir_leaf() {
    let sysfs = init_sysfs_with_mock_tree();
    let root_inode = sysfs.root_inode();
    let leaf1_inode = root_inode
        .lookup("branch1")
        .unwrap()
        .lookup("leaf1")
        .unwrap(); // The sysfs dir for the leaf node
    let mut visitor = TestDirentVisitor { entries: vec![] };

    // Action: Read directory entries from the sysfs directory representing a systree leaf node

    let mut offset = 0;
    loop {
        // Pass offset as usize
        let result = leaf1_inode.readdir_at(offset, &mut visitor);
        match result {
            Ok(next_offset) => {
                if next_offset == offset || next_offset == 0 {
                    // Check if no progress or end
                    break;
                }
                offset = next_offset;
            }
            Err(e) => {
                panic!("readdir_at failed unexpectedly: {:?}", e);
            }
        }
    }

    let mut names: Vec<_> = visitor.entries.iter().map(|(n, _, _)| n.clone()).collect();
    names.sort();

    assert!(names.contains(&".".to_string()));
    assert!(names.contains(&"..".to_string()));
    assert!(names.contains(&"r_attr1".to_string()));
    assert!(names.contains(&"rw_attr1".to_string()));

    for (name, _, type_) in &visitor.entries {
        match name.as_str() {
            "." | ".." => assert_eq!(*type_, InodeType::Dir),
            "r_attr1" | "rw_attr1" => assert_eq!(*type_, InodeType::File),
            _ => panic!("Unexpected entry: {}", name),
        }
    }
}

#[ktest]
fn test_sysfs_mode_permissions() {
    let sysfs = init_sysfs_with_mock_tree();
    let root_inode = sysfs.root_inode();
    let leaf1_dir_inode = root_inode
        .lookup("branch1")
        .unwrap()
        .lookup("leaf1")
        .unwrap();
    let r_attr_inode = leaf1_dir_inode.lookup("r_attr1").unwrap(); // Sysfs file for read-only attr
    let rw_attr_inode = leaf1_dir_inode.lookup("rw_attr1").unwrap(); // Sysfs file for read-write attr

    // Verification: Check that the default mode (permissions) of the sysfs files/dirs
    // correctly reflects the SysPerms of the underlying systree attributes/nodes.
    // Also test that set_mode works on the sysfs inode.

    // Check default modes based on SysPerms
    let r_mode = r_attr_inode.mode().unwrap();
    assert!(r_mode.contains(InodeMode::S_IRUSR | InodeMode::S_IRGRP | InodeMode::S_IROTH)); // 0o444
    assert!(!r_mode.contains(InodeMode::S_IWUSR)); // Not 0o200

    let rw_mode = rw_attr_inode.mode().unwrap();
    assert!(rw_mode.contains(InodeMode::S_IRUSR | InodeMode::S_IRGRP | InodeMode::S_IROTH)); // 0o444
    assert!(rw_mode.contains(InodeMode::S_IWUSR)); // 0o200

    // Test set_mode
    let new_mode = InodeMode::from_bits_truncate(0o600); // rw-------
    rw_attr_inode.set_mode(new_mode).expect("set_mode failed");
    assert_eq!(rw_attr_inode.mode().unwrap(), new_mode);

    // Directories should have default mode (e.g., 0o555)
    let leaf1_mode = leaf1_dir_inode.mode().unwrap();
    assert!(leaf1_mode.contains(InodeMode::S_IRUSR | InodeMode::S_IXUSR)); // Read/execute for user
    assert!(leaf1_mode.contains(InodeMode::S_IRGRP | InodeMode::S_IXGRP)); // Read/execute for group
    assert!(leaf1_mode.contains(InodeMode::S_IROTH | InodeMode::S_IXOTH)); // Read/execute for other
}
