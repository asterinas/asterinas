// SPDX-License-Identifier: MPL-2.0

use alloc::{
    borrow::ToOwned,
    string::String,
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
use core::{
    any::Any,
    fmt::Debug,
    sync::atomic::{AtomicU64, Ordering},
};

use bitflags::bitflags;
use ostd::mm::{VmReader, VmWriter};

use super::{Error, Result, SysAttrSet, SysStr};

pub const MAX_ATTR_SIZE: usize = 4096;

/// The three types of nodes in a `SysTree`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SysNodeType {
    /// A branching node is one that can have child nodes.
    Branch,
    /// A leaf node is one that cannot have child nodes.
    Leaf,
    /// A symlink node,
    /// which ia a special kind of leaf node that points to another node,
    /// similar to a symbolic link in file systems.
    Symlink,
}

/// A trait that represents a branching node in a `SysTree`.
#[expect(clippy::type_complexity)]
pub trait SysBranchNode: SysNode {
    /// Visits a child node with the given name using a closure.
    ///
    /// If the child with the given name exists,
    /// a reference to the child will be provided to the closure.
    /// Otherwise, the closure will be given a `None`.
    ///
    /// # Efficiency
    ///
    /// This method is a more efficient, but less convenient version
    /// of the `child` method.
    /// The method does not require taking the ownership of the child node.
    /// So use this method when efficiency is a primary concern,
    /// while using the `child` method for the sake of convenience.
    ///
    /// # Deadlock
    ///
    /// The implementation of this method depends on the concrete type
    /// and probably will hold an internal lock.
    /// So the caller should do as little as possible inside the closure.
    /// In particular, the caller should _not_ invoke other methods
    /// on this object as this might cause deadlock.
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    /// Visits child nodes with a minimum ID using a closure.
    ///
    /// This method iterates over the child nodes
    /// whose IDs are no less than a specified minimum value.
    /// and provide them to the given closure one at a time.
    ///
    /// The iteration terminates until there are no unvisited children
    /// or the closure returns a `None`.
    ///
    /// # Efficiency
    ///
    /// This method is a more efficient, but less convenient version
    /// of the `children` method.
    /// The method require neither taking the ownership of the child nodes
    /// nor doing heap allocations.
    /// So use this method when efficiency is a primary concern,
    /// while using the `children` method for the sake of convenience.
    ///
    /// # Deadlock
    ///
    /// Same as the `visit_child_with` method.
    fn visit_children_with(
        &self,
        min_id: u64,
        f: &mut dyn for<'a> FnMut(&'a Arc<(dyn SysObj)>) -> Option<()>,
    );

    /// Returns a child with a specified name.
    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    /// Collects all children into a `Vec`.
    fn children(&self) -> Vec<Arc<dyn SysObj>> {
        let mut children: Vec<Arc<dyn SysObj>> = Vec::new();
        self.visit_children_with(0, &mut |child_arc| {
            children.push(child_arc.clone());
            Some(())
        });
        children
    }

    /// Counts the number of children.
    fn count_children(&self) -> usize {
        let mut count = 0;
        self.visit_children_with(0, &mut |_| {
            count += 1;
            Some(())
        });
        count
    }

    /// Creates a new child node with the given name.
    ///
    /// This new child will be added to this branch node.
    fn create_child(&self, _name: &str) -> Result<Arc<dyn SysObj>> {
        Err(Error::PermissionDenied)
    }

    /// Removes a child node with the given name.
    fn remove_child(&self, _name: &str) -> Result<Arc<dyn SysObj>> {
        Err(Error::PermissionDenied)
    }
}

/// The trait that abstracts a "normal" node in a `SysTree`.
///
/// The branching and leaf nodes are considered "normal",
/// whereas the symlink nodes are considered "special".
/// This trait abstracts the common interface of "normal" nodes.
/// In particular, every "normal" node may have associated attributes.
pub trait SysNode: SysObj {
    /// Returns the attribute set of a `SysNode`.
    fn node_attrs(&self) -> &SysAttrSet;

    /// Reads the value of an attribute.
    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize>;

    /// Writes the value of an attribute.
    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize>;

    /// Reads the value of an attribute from the specified `offset`.
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize>;

    /// Writes the value of an attribute at the specified `offset`.
    fn write_attr_at(&self, name: &str, _offset: usize, reader: &mut VmReader) -> Result<usize>;

    /// Shows the string value of an attribute.
    ///
    /// Most attributes are textual, rather binary.
    /// So using this `show_attr` method is more convenient than
    /// the `read_attr` method.
    fn show_attr(&self, name: &str) -> Result<String> {
        let mut buf: Vec<u8> = vec![0; MAX_ATTR_SIZE];
        let mut writer = VmWriter::from(buf.as_mut_slice()).to_fallible();
        let read_len = self.read_attr(name, &mut writer)?;
        // Use from_utf8_lossy or handle error properly if strict UTF-8 is needed
        let attr_val =
            String::from_utf8(buf[..read_len].to_vec()).map_err(|_| Error::AttributeError)?;
        Ok(attr_val)
    }

    /// Stores the string value of an attribute.
    ///
    /// Most attributes are textual, rather binary.
    /// So using this `store_attr` method is more convenient than
    /// the `write_attr` method.
    fn store_attr(&self, name: &str, new_val: &str) -> Result<usize> {
        let mut reader = VmReader::from(new_val.as_bytes()).to_fallible();
        self.write_attr(name, &mut reader)
    }

    /// Returns the initial permissions of a node.
    ///
    /// The FS layer should take the value returned from this method
    /// as this initial permissions for the corresponding inode.
    fn perms(&self) -> SysPerms;
}

/// A trait that abstracts any symlink node in a `SysTree`.
pub trait SysSymlink: SysObj {
    /// A path that represents the target node of this symlink node.
    fn target_path(&self) -> &str;
}

/// The base trait for any node in a `SysTree`.
pub trait SysObj: Any + Send + Sync + Debug + 'static {
    /// Returns a reference to this object as `Any` for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Casts this object to a trait object of `SysTree` symlink.
    fn cast_to_symlink(&self) -> Option<Arc<dyn SysSymlink>> {
        None
    }

    /// Casts this object to a trait object of a `SysTree` node.
    fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
        None
    }

    /// Casts this object to a trait object of a `SysTree` branch node.
    fn cast_to_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
        None
    }

    /// Returns the unique and immutable ID of a node.
    fn id(&self) -> &SysNodeId;

    /// Returns the type of a node.
    fn type_(&self) -> SysNodeType;

    /// Returns the name of a node.
    ///
    /// The name is guaranteed _not_ to contain two special characters:
    /// `'/'` and `'\0'`.
    ///
    /// The root node of a `SysTree` has an empty name.
    /// All other inodes must have an non-empty name.
    fn name(&self) -> &SysStr;

    /// Returns whether a node is the root of a `SysTree`.
    fn is_root(&self) -> bool {
        false
    }

    /// Initializes the parent in the `SysTree`.
    ///
    /// An appropriate timing to call this method is when a `SysTree` node is added to
    /// another node as a child.
    ///
    /// # Panics
    ///
    /// This method should be called at most once; otherwise, it may trigger panicking.
    fn init_parent(&self, parent: Weak<dyn SysBranchNode>);

    /// Returns the parent in the `SysTree`.
    ///
    /// Returns `None` if the parent of the node has not been initialized yet.
    fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;

    /// Returns the path.
    ///
    /// The path of a `SysTree` node is defined as below:
    /// - If a node is the root, then the path is `/`.
    /// - If a node is a non-root, we will need to see if it has a parent:
    ///      - If the node has a parent, then the path is defined as
    ///        the parent path and the node's name joined with a `/` in between;
    ///      - Otherwise, the node's name is taken as its path.
    fn path(&self) -> SysStr {
        if self.is_root() {
            return SysStr::from("/");
        }

        let Some(parent) = self.parent() else {
            return self.name().clone();
        };

        let parent_path_with_slash = {
            let mut parent_path = parent.path().as_ref().to_owned();
            if !parent.is_root() {
                parent_path.push('/');
            }

            parent_path
        };

        SysStr::from(parent_path_with_slash + self.name())
    }
}

/// The unique ID of a `SysNode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SysNodeId(u64);

impl SysNodeId {
    /// Creates a new ID.
    pub fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);

        let next_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        // Guard against integer overflow
        assert!(next_id <= u64::MAX / 2);

        Self(next_id)
    }

    /// Gets the value of the ID.
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Default for SysNodeId {
    fn default() -> Self {
        Self::new()
    }
}

bitflags! {
    /// Permissions for a node or an attribute in the `SysTree`.
    ///
    /// This struct is mainly used to provide the initial permissions for nodes and attributes.
    ///
    /// The concepts of "owner"/"group"/"others" mentioned here are not explicitly represented in
    /// systree. They exist primarily to enable finer-grained permission management at
    /// the "view" and "control" parts for users. The definitions of these permissions match that
    /// of VFS file permissions bit-wise.
    ///
    /// Users can provide permission modification functionality through additional abstractions at
    /// the upper layers. Correspondingly, it is the users' responsibility to do the permission
    /// verification at the "view" and "control" parts.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct SysPerms: u16 {
        // One-bit flags:
        /// The read permission for owner.
        const OWNER_R = 0o0400;
        /// The write permission for owner.
        const OWNER_W = 0o0200;
        /// The execute/search permission for owner.
        const OWNER_X = 0o0100;
        /// The read permission for group.
        const GROUP_R = 0o0040;
        /// The write permission for group.
        const GROUP_W = 0o0020;
        /// The execute/search permission for group.
        const GROUP_X = 0o0010;
        /// The read permission for others.
        const OTHER_R = 0o0004;
        /// The write permission for others.
        const OTHER_W = 0o0002;
        /// The execute/search permission for others.
        const OTHER_X = 0o0001;

        // Common multi-bit flags:
        const ALL_R = 0o0444;
        const ALL_W = 0o0222;
        const ALL_X = 0o0111;
        const ALL_RW = 0o0666;
        const ALL_RX = 0o0555;
        const ALL_RWX = 0o0777;
    }
}

impl SysPerms {
    /// Default read-only permissions for nodes (owner/group/others can read+execute)
    pub const DEFAULT_RO_PERMS: Self = Self::ALL_RX;

    /// Default read-write permissions for nodes (owner has full, group/others read+execute)
    pub const DEFAULT_RW_PERMS: Self = Self::ALL_RX.union(Self::OWNER_W);

    /// Default read-only permissions for attributes (owner/group/others can read)
    pub const DEFAULT_RO_ATTR_PERMS: Self = Self::ALL_R;

    /// Default read-write permissions for attributes (owner read+write, group/others read)
    pub const DEFAULT_RW_ATTR_PERMS: Self = Self::ALL_R.union(Self::OWNER_W);

    /// Returns whether read operations are allowed.
    pub fn can_read(&self) -> bool {
        self.intersects(Self::ALL_R)
    }

    /// Returns whether write operations are allowed.
    pub fn can_write(&self) -> bool {
        self.intersects(Self::ALL_W)
    }
}
