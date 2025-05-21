// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;
use core::fmt::Debug;

use bitflags::bitflags;

use super::{Error, Result, SysStr};

bitflags! {
    /// Flags defining the properties and permissions of a `SysAttr`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct SysAttrFlags: u32 {
        /// Indicates whether the attribute can be read.
        const CAN_READ = 1 << 0;
        /// Indicates whether the attribute can be written to.
        const CAN_WRITE = 1 << 1;
        /// Indicates whether an attribute is a binary one
        /// (rather than a textual one).
        const IS_BINARY = 1 << 4;
    }
}

impl Default for SysAttrFlags {
    fn default() -> Self {
        Self::CAN_READ
    }
}

/// An attribute may be fetched or updated via the methods of `SysNode`
/// such as `SysNode::read_attr` and  `SysNode::write_attr`.
#[derive(Debug, Clone)]
pub struct SysAttr {
    /// Local ID within the node's `SysAttrSet`. Unique within the set.
    id: u8,
    /// The name of the attribute. Used to look up the attribute in a `SysAttrSet`.
    name: SysStr,
    /// Flags defining the behavior and permissions of the attribute.
    flags: SysAttrFlags,
    // Potentially add read/write handler functions or trait objects later
    // read_handler: fn(...) -> Result<usize>,
    // write_handler: fn(...) -> Result<usize>,
}

impl SysAttr {
    /// Creates a new attribute.
    pub fn new(id: u8, name: SysStr, flags: SysAttrFlags) -> Self {
        Self { id, name, flags }
    }

    /// Returns the unique ID of the attribute within its set.
    pub fn id(&self) -> u8 {
        self.id
    }

    /// Returns the name of the attribute.
    pub fn name(&self) -> &SysStr {
        &self.name
    }

    /// Returns the flags associated with the attribute.
    pub fn flags(&self) -> SysAttrFlags {
        self.flags
    }
}

/// A collection of `SysAttr` for a `SysNode`.
/// Manages the attributes associated with a specific node in the `SysTree`.
///
/// This is an immutable collection - use `SysAttrSetBuilder` to create non-empty sets.
#[derive(Debug, Default, Clone)]
pub struct SysAttrSet {
    /// Stores attributes keyed by their name.
    attrs: BTreeMap<SysStr, SysAttr>,
}

impl SysAttrSet {
    /// Maximum number of attributes allowed per node (limited by u8 ID space).
    pub const CAPACITY: usize = 1 << u8::BITS;

    /// Creates a new, empty attribute set.
    ///
    /// To create a non-empty attribute set, use `SysAttrSetBuilder`.
    pub fn new_empty() -> Self {
        Default::default()
    }

    /// Retrieves an attribute by its name.
    pub fn get(&self, name: &str) -> Option<&SysAttr> {
        self.attrs.get(name)
    }

    /// Returns an iterator over the attributes in the set.
    pub fn iter(&self) -> impl Iterator<Item = &SysAttr> {
        self.attrs.values()
    }

    /// Returns the number of attributes in the set.
    pub fn len(&self) -> usize {
        self.attrs.len()
    }

    /// Checks if the attribute set is empty.
    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty()
    }

    /// Checks if an attribute with the given name exists in the set.
    pub fn contains(&self, attr_name: &str) -> bool {
        self.attrs.contains_key(attr_name)
    }
}

#[derive(Debug, Default)]
pub struct SysAttrSetBuilder {
    attrs: BTreeMap<SysStr, SysAttr>,
    next_id: u8,
}

impl SysAttrSetBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Default::default()
    }

    /// Adds an attribute definition to the builder.
    ///
    /// If an attribute with the same name already exists, this is a no-op.
    pub fn add(&mut self, name: SysStr, flags: SysAttrFlags) -> &mut Self {
        if self.attrs.contains_key(&name) {
            return self;
        }

        let id = self.next_id;
        self.next_id += 1;
        let new_attr = SysAttr::new(id, name.clone(), flags);
        self.attrs.insert(name, new_attr);
        self
    }

    /// Consumes the builder and returns the constructed `SysAttrSet`.
    ///
    /// # Errors
    /// Returns `Err` if the capacity limit is reached.
    pub fn build(self) -> Result<SysAttrSet> {
        if self.attrs.len() > SysAttrSet::CAPACITY {
            return Err(Error::PermissionDenied);
        }
        Ok(SysAttrSet { attrs: self.attrs })
    }
}
