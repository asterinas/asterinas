// SPDX-License-Identifier: MPL-2.0

use super::{RouteAddressFamily, RouteEntry, RouteLookupKey, RouteMetric};
use crate::prelude::*;

/// A table manages multiple routes.
///
/// Routes should be organized in a prefix tree and selected by longest-prefix match.
//
// FIXME: The Rust ecosystem currently lacks a suitable trie-tree implementation,
// so entries are stored in a `BTreeMap`.
// Its key is used only for deduplication.
// Replace it with a trie tree when a suitable implementation becomes available.
pub(super) struct RouteTable<A: RouteAddressFamily> {
    entries: BTreeMap<(A::Cidr, RouteMetric), RouteEntry<A>>,
}

impl<A: RouteAddressFamily> RouteTable<A> {
    pub(super) const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub(super) fn insert(&mut self, entry: RouteEntry<A>) {
        self.entries.insert((*entry.dst(), entry.metric()), entry);
    }

    pub(super) fn lookup(&self, key: &RouteLookupKey<A>) -> Option<RouteEntry<A>> {
        let mut best: Option<&RouteEntry<A>> = None;

        for entry in self
            .entries
            .values()
            .filter(|entry| entry.matches_lookup(key))
        {
            let is_better = best.is_none_or(|best_entry| {
                let prefix_len = A::prefix_len(entry.dst());
                let best_prefix_len = A::prefix_len(best_entry.dst());
                prefix_len > best_prefix_len
                    || (prefix_len == best_prefix_len && entry.metric() < best_entry.metric())
            });

            if is_better {
                best = Some(entry);
            }
        }

        best.cloned()
    }
}
