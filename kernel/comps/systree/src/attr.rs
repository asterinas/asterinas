// SPDX-License-Identifier: MPL-2.0

#![expect(clippy::type_complexity)]

use alloc::{collections::BTreeMap, sync::Arc};
use core::fmt::Debug;

use bitflags::bitflags;
use ostd::mm::{VmReader, VmWriter};

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
#[derive(Clone)]
pub struct SysAttr {
    /// Local ID within the node's `SysAttrSet`. Unique within the set.
    id: u8,
    /// The name of the attribute. Used to look up the attribute in a `SysAttrSet`.
    name: SysStr,
    /// Flags defining the behavior and permissions of the attribute.
    flags: SysAttrFlags,
    /// Function to handle reading the attribute.
    read_handler: Arc<dyn Fn(&mut VmWriter) -> Result<usize> + Send + Sync>,
    /// Function to handle writing to the attribute.
    write_handler: Arc<dyn Fn(&mut VmReader) -> Result<usize> + Send + Sync>,
}

impl Debug for SysAttr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SysAttr")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("flags", &self.flags)
            .finish()
    }
}

impl SysAttr {
    /// Creates a new attribute.
    pub fn new(
        id: u8,
        name: SysStr,
        flags: SysAttrFlags,
        read_handler: impl Fn(&mut VmWriter) -> Result<usize> + Send + Sync + 'static,
        write_handler: impl Fn(&mut VmReader) -> Result<usize> + Send + Sync + 'static,
    ) -> Self {
        Self {
            id,
            name,
            flags,
            read_handler: Arc::new(read_handler),
            write_handler: Arc::new(write_handler),
        }
    }

    /// Reads the value of the attribute.
    pub fn read_attr(&self, writer: &mut VmWriter) -> Result<usize> {
        (self.read_handler)(writer)
    }

    /// Writes the value of the attribute.
    pub fn write_attr(&self, reader: &mut VmReader) -> Result<usize> {
        (self.write_handler)(reader)
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
    pub fn add(
        &mut self,
        name: SysStr,
        flags: SysAttrFlags,
        read_handler: impl Fn(&mut VmWriter) -> Result<usize> + Send + Sync + 'static,
        write_handler: impl Fn(&mut VmReader) -> Result<usize> + Send + Sync + 'static,
    ) -> &mut Self {
        if self.attrs.contains_key(&name) {
            return self;
        }

        let id = self.next_id;
        self.next_id += 1;
        let new_attr = SysAttr::new(id, name.clone(), flags, read_handler, write_handler);
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
