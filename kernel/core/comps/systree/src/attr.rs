// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::BTreeMap, sync::Arc};
use core::fmt::Debug;

use id_alloc::IdAlloc;
use spin::Once;

use super::{Error, Result, SysStr};
use crate::SysPerms;

/// An attribute may be fetched or updated via the methods of `SysNode`
/// such as `SysNode::read_attr` and  `SysNode::write_attr`.
#[derive(Clone, Debug)]
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

/// The attributes of one node in the `SysTree`.
///
/// A set is **immutable**: once built, its contents never change. A node whose
/// attributes come and go does not edit its set; it builds a new one and
/// publishes that instead, so that a reader always sees a whole set and never a
/// half-edited one. [`SysAttrSetBuilder::from_set`] is how the new set is
/// derived from the old, keeping the IDs of the attributes that stay.
///
/// Use [`SysAttrSetBuilder`] to create a set with an initial population.
#[derive(Clone, Debug, Default)]
pub struct SysAttrSet {
    /// Stores attributes keyed by their name.
    attrs: BTreeMap<SysStr, SysAttr>,
}

impl SysAttrSet {
    /// Maximum number of attributes allowed per node.
    pub const CAPACITY: usize = u8::MAX as usize;

    /// Creates a new, empty attribute set.
    ///
    /// To create a non-empty attribute set, use [`SysAttrSetBuilder`].
    pub const fn new_empty() -> Self {
        Self {
            attrs: BTreeMap::new(),
        }
    }

    /// Returns the shared empty set.
    ///
    /// Every node without attributes can point at this one, because a set is
    /// immutable.
    pub fn empty() -> &'static Arc<SysAttrSet> {
        static EMPTY: Once<Arc<SysAttrSet>> = Once::new();
        EMPTY.call_once(|| Arc::new(SysAttrSet::new_empty()))
    }

    /// Retrieves an attribute by its name.
    pub fn get(&self, name: &str) -> Option<&SysAttr> {
        self.attrs.get(name)
    }

    /// Returns an iterator over the attributes in the set, in name order.
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

/// Builds a [`SysAttrSet`].
///
/// The builder owns the ID space: [`Self::add`] takes the lowest ID that the
/// set being built is not already using, so a set derived from another with
/// [`Self::from_set`] keeps every surviving attribute at the ID it had, and an
/// ID freed by [`Self::remove`] can be taken by a later attribute.
#[derive(Debug)]
pub struct SysAttrSetBuilder {
    attrs: BTreeMap<SysStr, SysAttr>,
    /// The allocator for attribute IDs.
    ids: IdAlloc,
    /// The first error from [`Self::add`], returned by [`Self::build`].
    ///
    /// Storing the error keeps `add` chainable without silently building a
    /// partial attribute set after an addition fails.
    error: Option<Error>,
}

impl SysAttrSetBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self {
            attrs: BTreeMap::new(),
            ids: IdAlloc::with_capacity(SysAttrSet::CAPACITY),
            error: None,
        }
    }

    /// Creates a builder from `set`, keeping IDs stable for attributes that remain present.
    pub fn from_set(set: &SysAttrSet) -> Self {
        let mut builder = Self::new();
        for attr in set.iter() {
            builder.ids.alloc_specific(attr.id() as usize).unwrap();
            builder.attrs.insert(attr.name().clone(), attr.clone());
        }
        builder
    }

    /// Adds an attribute definition to the builder.
    ///
    /// If an attribute with the same name already exists, this is a no-op, so
    /// the existing attribute keeps its ID and its permissions.
    /// Invalid names and exhausted IDs are reported by [`Self::build`].
    pub fn add(&mut self, name: SysStr, perms: SysPerms) -> &mut Self {
        if self.error.is_some() {
            return self;
        }
        if !crate::is_valid_name(&name) {
            self.error = Some(Error::InvalidName);
            return self;
        }
        if self.attrs.contains_key(&name) {
            return self;
        }
        let Some(id) = self.ids.alloc() else {
            // `add` stays chainable, so exhaustion is reported by `build`.
            self.error = Some(Error::ResourceUnavailable);
            return self;
        };
        self.attrs
            .insert(name.clone(), SysAttr::new(id as u8, name, perms));
        self
    }

    /// Removes an attribute by name, freeing its ID. Absent names are ignored.
    pub fn remove(&mut self, name: &str) -> &mut Self {
        if let Some(attr) = self.attrs.remove(name) {
            self.ids.free(attr.id() as usize);
        }
        self
    }

    /// Consumes the builder and returns the constructed [`SysAttrSet`].
    ///
    /// # Errors
    ///
    /// Returns the first error from [`Self::add`]: [`Error::InvalidName`] for
    /// an invalid attribute name, or [`Error::ResourceUnavailable`] if all IDs
    /// were in use. Removing an attribute does not clear a previous error.
    /// A builder derived from a set that only removes attributes always succeeds.
    pub fn build(self) -> Result<SysAttrSet> {
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(SysAttrSet { attrs: self.attrs })
    }
}

impl Default for SysAttrSetBuilder {
    fn default() -> Self {
        Self::new()
    }
}
