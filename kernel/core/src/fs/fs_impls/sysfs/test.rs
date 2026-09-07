// SPDX-License-Identifier: MPL-2.0

use alloc::{borrow::Cow, format, vec};
use core::fmt::Debug;

use aster_systree::{
    BranchNodeFields, Error as SysTreeError, NormalNodeFields, ObjFields, Result as SysTreeResult,
    SymlinkNodeFields, SysAttrSet, SysAttrSetBuilder, SysBranchNode, SysNode, SysNodeId,
    SysNodeType, SysObj, SysPerms, SysStr, inherit_sys_branch_node, inherit_sys_leaf_node,
    inherit_sys_symlink_node, init_for_ktest,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::prelude::ktest;

use crate::{
    fs::{
        file::{InodeType, StatusFlags, mkmod},
        sysfs::fs::SysFs,
        utils::{DirentVisitor, systree_inode::SysTreeInodeTy},
        vfs::{file_system::FileSystem, path::Dentry},
    },
    prelude::*,
    time::clocks::init_for_ktest as time_init_for_ktest,
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
    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        let attr = self
            .fields
            .attr_set()
            .get(name)
            .ok_or(SysTreeError::NotFound)?;
        if !attr.perms().can_read() {
            return Err(SysTreeError::PermissionDenied);
        }
        let data = self.data.read();
        let value = data.get(name).ok_or(SysTreeError::NotFound)?; // Should exist if in attrs

        let mut printer = VmPrinter::new_skip(writer, offset);
        write!(printer, "{}", value)?;

        Ok(printer.bytes_written())
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
            .map_err(|_| SysTreeError::PageFault)?;

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
    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
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
            _ => return Err(SysTreeError::NotFound),
        };

        let mut printer = VmPrinter::new_skip(writer, offset);
        write!(printer, "{}", value)?;

        Ok(printer.bytes_written())
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

// A leaf node that allows changing its attribute set between lookups.
#[derive(Debug)]
struct MockMutableAttrNode {
    fields: ObjFields<Self>,
    attrs: RwLock<Arc<SysAttrSet>>,
}

impl MockMutableAttrNode {
    fn new() -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            fields: ObjFields::new("device".into(), this.clone()),
            attrs: RwLock::new(SysAttrSet::empty().clone()),
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for MockMutableAttrNode {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
        self.fields.weak_self().upgrade().map(|node| node as _)
    }

    fn type_(&self) -> SysNodeType {
        SysNodeType::Leaf
    }

    fn id(&self) -> &SysNodeId;
    fn name(&self) -> &SysStr;
    fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;
}

impl SysNode for MockMutableAttrNode {
    fn node_attrs(&self) -> Arc<SysAttrSet> {
        self.attrs.read().clone()
    }

    fn is_attr_absent(&self, _name: &str) -> bool {
        false
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> SysTreeResult<usize> {
        self.read_attr_at(name, 0, writer)
    }

    fn read_attr_at(
        &self,
        _name: &str,
        _offset: usize,
        _writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        Err(SysTreeError::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> SysTreeResult<usize> {
        Err(SysTreeError::AttributeError)
    }

    fn write_attr_at(
        &self,
        name: &str,
        _offset: usize,
        reader: &mut VmReader,
    ) -> SysTreeResult<usize> {
        self.write_attr(name, reader)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
}

// --- Test Setup ---

struct TestSysFs {
    fs: Arc<SysFs>,
    root_dentry: Arc<Dentry>,
}

/// Creates a sysfs view of the given node tree and returns its test fixture.
fn init_sysfs(root_node: Arc<dyn SysBranchNode>) -> TestSysFs {
    time_init_for_ktest();
    init_for_ktest();
    let fs = SysFs::new_for_ktest(root_node);
    let root_dentry = Dentry::new_root(fs.root_inode());
    TestSysFs { fs, root_dentry }
}

/// Creates a mock node tree with branches, leaves, and a symlink, returning its root node.
fn create_mock_systree() -> Arc<MockBranchNode> {
    let root = MockBranchNode::new("root");
    let branch1 = MockBranchNode::new("branch1");
    let leaf1 = MockLeafNode::new("leaf1".into(), &["r_attr1"], &["rw_attr1"]);
    let leaf2 = MockLeafNode::new("leaf2".into(), &["r_attr2"], &[]);
    let symlink1 = MockSymlinkNode::new("link1".into(), "../branch1/leaf1");

    branch1.add_child(leaf1);
    root.add_child(branch1);
    root.add_child(leaf2);
    root.add_child(symlink1);
    root
}

#[ktest]
fn root_lookup() {
    let test_fs = init_sysfs(create_mock_systree());
    let root_dentry = test_fs.root_dentry.clone();
    let expected_fs: Arc<dyn FileSystem> = test_fs.fs.clone();
    let root_inode = root_dentry.inode();

    assert_eq!(root_inode.type_(), InodeType::Dir);
    assert!(Arc::ptr_eq(&root_inode.fs(), &expected_fs));

    // Lookup existing branch
    let branch1_inode = root_inode.lookup("branch1").expect("Lookup branch1 failed");
    assert_eq!(branch1_inode.type_(), InodeType::Dir);
    assert!(Arc::ptr_eq(&branch1_inode.fs(), &expected_fs));

    // Lookup existing leaf (represented as Dir in sysfs)
    let leaf2_inode = root_inode.lookup("leaf2").expect("Lookup leaf2 failed");
    assert_eq!(leaf2_inode.type_(), InodeType::Dir);
    assert!(Arc::ptr_eq(&leaf2_inode.fs(), &expected_fs));

    // Lookup existing symlink
    let link1_inode = root_inode.lookup("link1").expect("Lookup link1 failed");
    assert_eq!(link1_inode.type_(), InodeType::SymLink);
    assert!(Arc::ptr_eq(&link1_inode.fs(), &expected_fs));

    let error = root_inode
        .create(&root_dentry, "new_node", InodeType::Dir, mkmod!(a+rx, u+w))
        .expect_err("creating an inode in sysfs should fail");
    assert_eq!(error.error(), Errno::EPERM);

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
fn branch_lookup() {
    let test_fs = init_sysfs(create_mock_systree());
    let root_dentry = test_fs.root_dentry.clone();
    let root_inode = root_dentry.inode();
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
fn leaf_lookup() {
    let test_fs = init_sysfs(create_mock_systree());
    let root_dentry = test_fs.root_dentry.clone();
    let root_inode = root_dentry.inode();
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
fn read_attr() {
    let test_fs = init_sysfs(create_mock_systree());
    let root_dentry = test_fs.root_dentry.clone();
    let root_inode = root_dentry.inode();
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
        .read_at(0, &mut writer, StatusFlags::empty())
        .expect("read_at failed");

    assert!(bytes_read > 0);
    let content = core::str::from_utf8(&buf[..bytes_read]).unwrap();
    assert_eq!(content, "val_r_attr1");

    // Reading a directory should fail (expect EINVAL as per inode.rs)
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible(); // Reset writer
    let result = leaf1_dir_inode.read_at(0, &mut writer, StatusFlags::empty());
    assert!(result.is_err());
}

#[ktest]
fn write_attr() {
    let test_fs = init_sysfs(create_mock_systree());
    let root_dentry = test_fs.root_dentry.clone();
    let root_inode = root_dentry.inode();
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
        .write_at(0, &mut reader, StatusFlags::empty())
        .expect("write_at failed");
    assert_eq!(bytes_written, new_val.len());

    // Read back to verify
    let mut buf = [0u8; 64];
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    let bytes_read = rw_attr_inode
        .read_at(0, &mut writer, StatusFlags::empty())
        .expect("read_at failed");
    let content = core::str::from_utf8(&buf[..bytes_read]).unwrap();
    assert_eq!(content, new_val);

    // Write to r_attr1 (should fail - EIO expected from underlying PermissionDenied)
    let mut reader = VmReader::from("attempt_write".as_bytes()).to_fallible();
    let result = r_attr_inode.write_at(0, &mut reader, StatusFlags::empty());
    assert!(result.is_err());

    // Writing to a directory should fail (expect EINVAL as per inode.rs)
    let mut reader = VmReader::from("attempt_write".as_bytes()).to_fallible();
    let result = leaf1_dir_inode.write_at(0, &mut reader, StatusFlags::empty());
    assert!(result.is_err());
}

#[ktest]
fn read_link() {
    use crate::fs::vfs::inode::SymbolicLink;

    let test_fs = init_sysfs(create_mock_systree());
    let root_dentry = test_fs.root_dentry.clone();
    let root_inode = root_dentry.inode();
    // Action: Lookup the sysfs symlink corresponding to a systree symlink node
    let link1_inode = root_inode.lookup("link1").unwrap();

    // Verification: Read the sysfs symlink and check if the target path matches
    // the path provided by the underlying mock systree symlink node's target_path method.

    let target = link1_inode.read_link().expect("read_link failed");
    assert!(matches!(
        target,
        SymbolicLink::Plain(s) if s == "../branch1/leaf1"
    ));

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
fn readdir_leaf() {
    let test_fs = init_sysfs(create_mock_systree());
    let root_dentry = test_fs.root_dentry.clone();
    let root_inode = root_dentry.inode();
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
fn mode_permissions() {
    let test_fs = init_sysfs(create_mock_systree());
    let root_dentry = test_fs.root_dentry.clone();
    let root_inode = root_dentry.inode();
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
    assert!(r_mode.contains(mkmod!(a+r))); // 0o444
    assert!(!r_mode.contains(mkmod!(u+w))); // Not 0o200

    let rw_mode = rw_attr_inode.mode().unwrap();
    assert!(rw_mode.contains(mkmod!(a+r))); // 0o444
    assert!(rw_mode.contains(mkmod!(u+w))); // 0o200

    // Test set_mode
    let new_mode = mkmod!(u+rw); // rw-------
    let rw_attr_dentry = Dentry::new_root(rw_attr_inode.clone());
    rw_attr_inode
        .set_mode(&rw_attr_dentry, new_mode)
        .expect("set_mode failed");
    assert_eq!(rw_attr_inode.mode().unwrap(), new_mode);

    // Directories should have default mode (e.g., 0o555)
    let leaf1_mode = leaf1_dir_inode.mode().unwrap();
    assert!(leaf1_mode.contains(mkmod!(a+rx))); // Read/execute for all users
}

#[ktest]
fn cached_child_lookup_observes_tree_changes() {
    // 1. Create a directory with no child nodes and a VFS dentry for it.
    let branch = MockBranchNode::new("root");
    let test_fs = init_sysfs(branch.clone());
    let root_dentry = test_fs.root_dentry.clone();
    let dir = root_dentry.as_dir_dentry_or_err().unwrap();

    // 2. Look up the missing child, caching its absence (a negative dentry).
    assert_eq!(
        dir.lookup_child("child").unwrap_err().error(),
        Errno::ENOENT
    );

    // 3. Attach a leaf. Lookup must discard the cached absence and find it.
    // A SysTree leaf appears as a directory containing its attributes in sysfs.
    branch.add_child(MockLeafNode::new("child".into(), &[], &[]));
    let leaf = dir.lookup_child("child").unwrap();
    assert_eq!(leaf.inode().type_(), InodeType::Dir);
    // Looking up the unchanged child again must reuse the positive dentry.
    assert!(Arc::ptr_eq(&leaf, &dir.lookup_child("child").unwrap()));

    // 4. Remove the leaf. Lookup must discard the cached child and return ENOENT.
    branch.fields.remove_child("child").unwrap();
    assert_eq!(
        dir.lookup_child("child").unwrap_err().error(),
        Errno::ENOENT
    );

    // 5. Attach a branch under the same name. Lookup must find the new inode.
    branch.add_child(MockBranchNode::new("child"));
    let child_dir = dir.lookup_child("child").unwrap();
    assert_ne!(child_dir.inode().ino(), leaf.inode().ino());

    // 6. Replace the branch with a symlink, with no lookup in between.
    // The name still exists, but the cached directory must be replaced.
    branch.fields.remove_child("child").unwrap();
    branch.add_child(MockSymlinkNode::new("child".into(), "target"));
    let link = dir.lookup_child("child").unwrap();
    assert_eq!(link.inode().type_(), InodeType::SymLink);
    assert_ne!(link.inode().ino(), child_dir.inode().ino());
    assert!(Arc::ptr_eq(&link, &dir.lookup_child("child").unwrap()));

    // 7. Remove the symlink. Its positive dentry must also be invalidated.
    branch.fields.remove_child("child").unwrap();
    assert_eq!(
        dir.lookup_child("child").unwrap_err().error(),
        Errno::ENOENT
    );
}

#[ktest]
fn cached_attr_lookup_observes_snapshot_changes() {
    // 1. Create a device directory with an empty attribute snapshot.
    let branch = MockBranchNode::new("root");
    let node = MockMutableAttrNode::new();
    branch.add_child(node.clone());
    let test_fs = init_sysfs(branch);
    let root_dentry = test_fs.root_dentry.clone();
    let leaf = root_dentry
        .as_dir_dentry_or_err()
        .unwrap()
        .lookup_child("device")
        .unwrap();
    let dir = leaf.as_dir_dentry_or_err().unwrap();

    // 2. Look up the missing attribute to cache its absence.
    assert_eq!(
        dir.lookup_child("status").unwrap_err().error(),
        Errno::ENOENT
    );

    // 3. Publish a snapshot containing status. Lookup must find the new file.
    let attrs = {
        let mut builder = SysAttrSetBuilder::new();
        builder.add("status".into(), SysPerms::DEFAULT_RO_ATTR_PERMS);
        Arc::new(builder.build().unwrap())
    };
    *node.attrs.write() = attrs.clone();
    let attr = dir.lookup_child("status").unwrap();
    assert_eq!(attr.inode().type_(), InodeType::File);
    // With the snapshot unchanged, the next lookup must reuse the cached file.
    assert!(Arc::ptr_eq(&attr, &dir.lookup_child("status").unwrap()));

    // 4. Publish an empty snapshot. The cached file must now disappear.
    *node.attrs.write() = SysAttrSet::empty().clone();
    assert_eq!(
        dir.lookup_child("status").unwrap_err().error(),
        Errno::ENOENT
    );

    // 5. Restore the snapshot. The cached absence must be replaced by a new dentry.
    *node.attrs.write() = attrs;
    let restored = dir.lookup_child("status").unwrap();
    assert!(!Arc::ptr_eq(&attr, &restored));
    assert_eq!(restored.inode().type_(), InodeType::File);
}

#[ktest]
fn cached_attr_lookup_observes_permission_changes() {
    // 1. Create a device with a read-only status attribute.
    let branch = MockBranchNode::new("root");
    let node = MockMutableAttrNode::new();
    let attrs = {
        let mut builder = SysAttrSetBuilder::new();
        builder.add("status".into(), SysPerms::DEFAULT_RO_ATTR_PERMS);
        Arc::new(builder.build().unwrap())
    };
    *node.attrs.write() = attrs;
    branch.add_child(node.clone());
    let test_fs = init_sysfs(branch);
    let root_dentry = test_fs.root_dentry.clone();
    let leaf = root_dentry
        .as_dir_dentry_or_err()
        .unwrap()
        .lookup_child("device")
        .unwrap();
    let dir = leaf.as_dir_dentry_or_err().unwrap();

    // 2. Cache the attribute and check its initial mode (0444).
    let attr = dir.lookup_child("status").unwrap();
    assert_eq!(attr.inode().mode().unwrap(), mkmod!(a+r));

    // 3. chmod the cached inode to 0400, leaving the attribute definition unchanged.
    // The next lookup must reuse that inode and preserve the chmod result.
    attr.inode().set_mode(&attr, mkmod!(u+r)).unwrap();
    let cached = dir.lookup_child("status").unwrap();
    assert!(Arc::ptr_eq(&attr, &cached));
    assert_eq!(cached.inode().mode().unwrap(), mkmod!(u+r));

    // 4. Replace the attribute definition with mode 0644, reusing its name and ID.
    // Publish only the final snapshot, so lookup never sees the attribute absent.
    let old_attrs = node.node_attrs();
    let new_attrs = {
        let mut builder = SysAttrSetBuilder::from_set(&old_attrs);
        builder.remove("status");
        builder.add("status".into(), SysPerms::DEFAULT_RW_ATTR_PERMS);
        Arc::new(builder.build().unwrap())
    };
    assert_eq!(
        old_attrs.get("status").unwrap().id(),
        new_attrs.get("status").unwrap().id()
    );
    *node.attrs.write() = new_attrs;

    // 5. Lookup must discard the old dentry and return the new definition's mode.
    let updated = dir.lookup_child("status").unwrap();
    assert!(!Arc::ptr_eq(&attr, &updated));
    assert_eq!(updated.inode().mode().unwrap(), mkmod!(u+rw, a+r));
}
