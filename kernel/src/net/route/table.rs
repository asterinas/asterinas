// SPDX-License-Identifier: MPL-2.0

use super::{RouteEntry, RouteLookupKey};
use crate::prelude::*;

/// One Linux route table.
///
/// Although the type is named as a table, it intentionally stores routes in a
/// `Vec` rather than a key-value map. Route lookup currently needs
/// longest-prefix selection, and rtnetlink dumps must enumerate entries. The
/// current per-table route count is small, so a linear scan keeps those
/// semantics explicit without prematurely choosing an index structure.
#[derive(Clone, Debug)]
pub(super) struct RouteTable {
    entries: Vec<RouteEntry>,
}

impl RouteTable {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(super) fn entries(&self) -> &[RouteEntry] {
        &self.entries
    }

    pub(super) fn insert(&mut self, route: RouteEntry) {
        self.entries.push(route);
    }

    pub(super) fn lookup_with_key(&self, key: &RouteLookupKey) -> Option<RouteEntry> {
        let mut best = None;
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.matches_lookup(key))
        {
            if best
                .is_none_or(|best: &RouteEntry| entry.dst().prefix_len() > best.dst().prefix_len())
            {
                best = Some(entry);
            }
        }

        best.cloned()
    }
}
