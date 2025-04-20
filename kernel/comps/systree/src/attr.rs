// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;
use core::fmt::Debug;

use bitflags::bitflags;

use super::{Error, Result, SysStr};

// SysAttrFlags definition from original lib.rs
bitflags! {
    /// Flags defining the properties and permissions of a `SysAttr`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct SysAttrFlags: u32 {
        /// Indicates whether the attribute can be read.
        const CAN_READ = 1 << 0;
        /// Indicates whether the attribute can be written to.
        const CAN_WRITE = 1 << 1;
    }
}

impl Default for SysAttrFlags {
    fn default() -> Self {
        Self::CAN_READ
    }
}

/// Represents an attribute (like a file) associated with a `SysNode`.
/// Attributes define the readable/writable properties of a node.
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
#[derive(Debug, Default)]
pub struct SysAttrSet {
    /// Stores attributes keyed by their name.
    attrs: BTreeMap<SysStr, SysAttr>,
    /// Counter to assign unique IDs to new attributes within this set.
    next_id: u8,
}

impl SysAttrSet {
    /// Maximum number of attributes allowed per node (limited by the u8 ID).
    pub const CAPACITY: usize = 256;

    /// Creates a new, empty attribute set.
    pub fn new() -> Self {
        Default::default()
    }

    /// Adds a new attribute to the set.
    ///
    /// # Arguments
    /// * `name` - The name of the new attribute. Must be unique within the set.
    /// * `flags` - The flags for the new attribute.
    ///
    /// # Errors
    /// Returns `Err` if an attribute with the same name already exists or
    /// if the capacity limit is reached.
    pub fn add(&mut self, name: SysStr, flags: SysAttrFlags) -> Result<()> {
        if self.attrs.contains_key(&name) {
            return Err(Error);
        }
        if self.attrs.len() >= Self::CAPACITY {
            return Err(Error);
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(Error)?;
        let attr = SysAttr::new(id, name.clone(), flags);
        self.attrs.insert(name, attr);
        Ok(())
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

/// A helper to construct a `SysAttrSet`.
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
    /// Skips adding if an attribute with the same name already exists or capacity is reached.
    /// Returns `Ok` on success, `Err` on failure (e.g., capacity).
    pub fn add(&mut self, name: SysStr, flags: SysAttrFlags) -> Result<&mut Self> {
        if self.attrs.contains_key(&name) {
            return Ok(self);
        }
        if self.attrs.len() >= SysAttrSet::CAPACITY {
            return Err(Error);
        }

        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(Error)?;
        let new_attr = SysAttr::new(id, name.clone(), flags);
        self.attrs.insert(name, new_attr);
        Ok(self)
    }

    /// Consumes the builder and returns the constructed `SysAttrSet`.
    pub fn build(self) -> SysAttrSet {
        SysAttrSet {
            attrs: self.attrs,
            next_id: self.next_id,
        }
    }
}
