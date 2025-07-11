// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;
use core::fmt::Debug;

use super::{Error, Result, SysStr};
use crate::SysPerms;

/// An attribute may be fetched or updated via the methods of `SysNode`
/// such as `SysNode::read_attr` and  `SysNode::write_attr`.
#[derive(Debug, Clone)]
pub struct SysAttr {
    /// Local ID within the node's `SysAttrSet`. Unique within the set.
    id: u8,
    /// The name of the attribute. Used to look up the attribute in a `SysAttrSet`.
    name: SysStr,
    /// The initial permissions of the attribute.
    perms: SysPerms,
}

impl SysAttr {
    /// Creates a new attribute.
    pub fn new(id: u8, name: SysStr, perms: SysPerms) -> Self {
        Self { id, name, perms }
    }

    /// Returns the unique ID of the attribute within its set.
    pub fn id(&self) -> u8 {
        self.id
    }

    /// Returns the name of the attribute.
    pub fn name(&self) -> &SysStr {
        &self.name
    }

    /// Returns the [`SysPerms`] representing the initial permissions of the attribute.
    pub fn perms(&self) -> SysPerms {
        self.perms
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
    pub const fn new_empty() -> Self {
        Self {
            attrs: BTreeMap::new(),
        }
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
    pub fn add(&mut self, name: SysStr, perms: SysPerms) -> &mut Self {
        if self.attrs.contains_key(&name) {
            return self;
        }

        let id = self.next_id;
        self.next_id += 1;
        let new_attr = SysAttr::new(id, name.clone(), perms);
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
