// SPDX-License-Identifier: MPL-2.0

use alloc::{string::String, sync::Arc, vec, vec::Vec};
use core::{
    any::Any,
    fmt::Debug,
    sync::atomic::{AtomicU64, Ordering},
};

use ostd::mm::{VmReader, VmWriter};

use super::{Error, Result, SysAttrSet, SysStr};

pub const MAX_ATTR_SIZE: usize = 4096;

/// The three types of nodes in a `SysTree`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SysNodeType {
    /// A branching node is one that may contain child nodes.
    Branch,
    /// A leaf node is one that may not contain child nodes.
    Leaf,
    /// A symlink node,
    /// which ia a special kind of leaf node that points to another node,
    /// similar to a symbolic link in file systems.
    Symlink,
}

/// A trait that represents a branching node in a `SysTree`.
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
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&dyn SysNode>));

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
        f: &mut dyn for<'a> FnMut(&'a Arc<(dyn SysObj + 'static)>) -> Option<()>,
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

    /// Shows the string value of an attribute.
    ///
    /// Most attributes are textual, rather binary (see `SysAttrFlags::IS_BINARY`).
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
    /// Most attributes are textual, rather binary (see `SysAttrFlags::IS_BINARY`).
    /// So using this `store_attr` method is more convenient than
    /// the `write_attr` method.
    fn store_attr(&self, name: &str, new_val: &str) -> Result<usize> {
        let mut reader = VmReader::from(new_val.as_bytes()).to_fallible();
        self.write_attr(name, &mut reader)
    }
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

    /// Attempts to get an Arc to this object as a `SysSymlink`.
    fn arc_as_symlink(&self) -> Option<Arc<dyn SysSymlink>> {
        None
    }

    /// Attempts to get an Arc to this object as a `SysNode`.
    fn arc_as_node(&self) -> Option<Arc<dyn SysNode>> {
        None
    }

    /// Attempts to get an Arc to this object as a `SysBranchNode`.
    fn arc_as_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
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
    fn name(&self) -> SysStr;

    /// Returns whether a node is the root of a `SysTree`.
    fn is_root(&self) -> bool {
        false
    }

    /// Returns the path from the root to this node.
    ///
    /// The path of a node is the names of all the ancestors concatenated
    /// with `/` as the separator.
    ///
    /// If the node has been attached to a `SysTree`,
    /// then the returned path begins with `/`.
    /// Otherwise, the returned path does _not_ begin with `/`.
    fn path(&self) -> SysStr {
        todo!("implement with the parent and name methods")
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
