// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use core::fmt::Debug;

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
    /// Maximum number of attributes allowed per node (limited by u8 ID space).
    pub const CAPACITY: usize = 1 << u8::BITS;

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

    /// Returns the attributes in the set, in name order.
    pub fn to_vec(&self) -> Vec<SysAttr> {
        self.attrs.values().cloned().collect()
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

/// The number of `u64` words needed to hold one bit per attribute ID.
const ID_WORDS: usize = SysAttrSet::CAPACITY / u64::BITS as usize;

/// Builds a [`SysAttrSet`].
///
/// The builder owns the ID space: [`Self::add`] takes the lowest ID that the
/// set being built is not already using, so a set derived from another with
/// [`Self::from_set`] keeps every surviving attribute at the ID it had, and an
/// ID freed by [`Self::remove`] can be taken by a later attribute.
#[derive(Debug, Default)]
pub struct SysAttrSetBuilder {
    attrs: BTreeMap<SysStr, SysAttr>,
    /// A bitmap of the IDs already taken.
    ids: [u64; ID_WORDS],
    /// How many attributes [`Self::add`] had to drop for want of an ID.
    dropped: usize,
}

impl SysAttrSetBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Default::default()
    }

    /// Creates a builder holding the attributes of `set`, with their IDs.
    pub fn from_set(set: &SysAttrSet) -> Self {
        let mut builder = Self::new();
        for attr in set.iter() {
            builder.take_id(attr.id());
            builder.attrs.insert(attr.name().clone(), attr.clone());
        }
        builder
    }

    /// Adds an attribute definition to the builder.
    ///
    /// If an attribute with the same name already exists, this is a no-op, so
    /// the existing attribute keeps its ID and its permissions.
    pub fn add(&mut self, name: SysStr, perms: SysPerms) -> &mut Self {
        if self.attrs.contains_key(&name) {
            return self;
        }
        let Some(id) = self.allocate_id() else {
            // `add` stays chainable, so exhaustion is reported by `build`.
            self.dropped += 1;
            return self;
        };
        self.attrs
            .insert(name.clone(), SysAttr::new(id, name, perms));
        self
    }

    /// Removes an attribute by name, freeing its ID. Absent names are ignored.
    pub fn remove(&mut self, name: &str) -> &mut Self {
        if let Some(attr) = self.attrs.remove(name) {
            self.free_id(attr.id());
        }
        self
    }

    /// Consumes the builder and returns the constructed [`SysAttrSet`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::ResourceUnavailable`] if [`Self::add`] ran out of IDs
    /// and dropped an attribute. A later [`Self::remove`] does not undo that:
    /// the caller asked for an attribute that is not in the set, and should
    /// hear so. Nothing else can fail, so a builder that only removes
    /// attributes always succeeds.
    pub fn build(self) -> Result<SysAttrSet> {
        if self.dropped > 0 {
            return Err(Error::ResourceUnavailable);
        }
        Ok(SysAttrSet { attrs: self.attrs })
    }

    fn allocate_id(&mut self) -> Option<u8> {
        for (word_idx, word) in self.ids.iter_mut().enumerate() {
            if *word == u64::MAX {
                continue;
            }
            let bit = word.trailing_ones();
            *word |= 1 << bit;
            return Some((word_idx as u32 * u64::BITS + bit) as u8);
        }
        None
    }

    fn take_id(&mut self, id: u8) {
        let id = id as u32;
        self.ids[(id / u64::BITS) as usize] |= 1u64 << (id % u64::BITS);
    }

    fn free_id(&mut self, id: u8) {
        let id = id as u32;
        self.ids[(id / u64::BITS) as usize] &= !(1u64 << (id % u64::BITS));
    }
}
