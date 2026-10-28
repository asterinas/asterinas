// SPDX-License-Identifier: MPL-2.0

use super::node::ExtentNode;
use crate::fs::fs_impls::ext4::{fs::Ext4, prelude::*};

/// Inode-private extent nodes with per-node dirty tracking.
///
/// The mapping lock excludes edits during writeback. Cache bookkeeping is
/// protected by the inner mutex, which is never held during device I/O.
/// Clean nodes may be evicted; dirty nodes remain authoritative until written
/// or explicitly retired by a mapping update.
#[derive(Debug)]
pub(super) struct ExtentNodeCache {
    inner: Mutex<CacheInner>,
}

#[derive(Debug, Default)]
struct CacheInner {
    nodes: BTreeMap<Ext4Bid, CachedExtentNode>,
    clean_lru: VecDeque<Ext4Bid>,
}

/// Decoded persistent data, separate from the cache's indexing and eviction state.
#[derive(Debug)]
struct CachedExtentNode {
    data: Dirty<Arc<ExtentNode>>,
}

/// An immutable version retained until all submitted I/O has completed.
///
/// The snapshot does not clear dirty state on drop: failed writeback leaves the
/// cache authoritative. Completion only cleans the exact version submitted.
struct NodeWriteback {
    bid: Ext4Bid,
    node: Arc<ExtentNode>,
}

impl ExtentNodeCache {
    pub(super) fn new() -> Self {
        Self {
            inner: Mutex::new(CacheInner::default()),
        }
    }

    pub(super) fn load(
        &self,
        fs: &Ext4,
        bid: Ext4Bid,
        expected_depth: u16,
    ) -> Result<Arc<ExtentNode>> {
        let hit = {
            let mut cache = self.inner.lock();
            let hit = cache.nodes.get(&bid).map(|entry| entry.data.clone());
            if hit.is_some() && cache.clean_lru.contains(&bid) {
                cache.touch_clean(bid);
            }
            hit
        };
        if let Some(node) = hit {
            if node.depth != expected_depth {
                return_errno_with_message!(Errno::EUCLEAN, "cached extent child depth mismatch");
            }
            return Ok(node);
        }
        let bytes = fs
            .block_device()
            .read_val::<[u8; BLOCK_SIZE]>(Bid::new(bid.into()).to_offset())?;
        let node = Arc::new(ExtentNode::parse(&bytes, Some(expected_depth))?);
        node.validate_addresses(fs)?;
        let mut cache = self.inner.lock();
        // Concurrent readers may have filled the same immutable version.
        let entry = cache.nodes.entry(bid).or_insert_with(|| CachedExtentNode {
            data: Dirty::new(node),
        });
        let node = entry.data.clone();
        cache.touch_clean(bid);
        Ok(node)
    }

    pub(super) fn stage(&mut self, bid: Ext4Bid, node: Arc<ExtentNode>) {
        let mut cache = self.inner.lock();
        cache.clean_lru.retain(|key| *key != bid);
        if let Some(entry) = cache.nodes.get_mut(&bid) {
            *entry.data = node;
        } else {
            cache.nodes.insert(
                bid,
                CachedExtentNode {
                    data: Dirty::_new_dirty(node),
                },
            );
        }
    }

    pub(super) fn remove(&mut self, bid: Ext4Bid) {
        let mut cache = self.inner.lock();
        cache.clean_lru.retain(|key| *key != bid);
        if let Some(mut entry) = cache.nodes.remove(&bid) {
            // The accepted mapping update retired this block; writing the old
            // contents after reuse would be incorrect.
            entry.data.clear_dirty();
        }
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.inner
            .lock()
            .nodes
            .values()
            .any(|entry| entry.data.is_dirty())
    }

    pub(super) fn sync(&self, fs: &Ext4) -> Result<()> {
        let mut pending: Vec<_> = self
            .inner
            .lock()
            .nodes
            .iter()
            .filter(|(_, entry)| entry.data.is_dirty())
            .map(|(&bid, entry)| NodeWriteback {
                bid,
                node: entry.data.clone(),
            })
            .collect();
        pending.sort_by_key(|snapshot| snapshot.node.depth);
        // Complete child nodes before submitting their parents.
        for snapshot in &pending {
            let mut bytes = Box::new([0u8; BLOCK_SIZE]);
            snapshot.node.write_into(bytes.as_mut());
            fs.block_device().write_bytes(
                Bid::new(u64::from(snapshot.bid)).to_offset(),
                bytes.as_ref(),
            )?;
            self.inner.lock().complete(snapshot);
        }
        Ok(())
    }
}

// Each external node occupies one filesystem block.
const MAX_CLEAN_NODES: usize = 32;

impl CacheInner {
    fn touch_clean(&mut self, bid: Ext4Bid) {
        debug_assert!(!self.nodes[&bid].data.is_dirty());
        self.clean_lru.retain(|key| *key != bid);
        if self.clean_lru.len() == MAX_CLEAN_NODES {
            let evicted = self.clean_lru.pop_front().unwrap();
            self.nodes.remove(&evicted);
        }
        self.clean_lru.push_back(bid);
    }

    fn complete(&mut self, snapshot: &NodeWriteback) {
        let Some(entry) = self.nodes.get_mut(&snapshot.bid) else {
            return;
        };
        if !Arc::ptr_eq(&entry.data, &snapshot.node) {
            return;
        }
        entry.data.clear_dirty();
        self.touch_clean(snapshot.bid);
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;
    use crate::fs::ext4::inode::block_mapping::extent::empty_extent_root;

    #[ktest]
    fn stale_writeback_completion_preserves_replacement_until_its_own_completion() {
        let root = empty_extent_root();
        let first = Arc::new(ExtentNode::parse(root.as_bytes(), None).unwrap());
        let mut next = (*first).clone();
        next.capacity -= 1;
        let next = Arc::new(next);
        let mut cache = ExtentNodeCache::new();
        cache.stage(40, first.clone());
        let older = NodeWriteback {
            bid: 40,
            node: first,
        };
        cache.stage(40, next.clone());
        cache.inner.lock().complete(&older);
        assert!(cache.is_dirty());
        {
            let inner = cache.inner.lock();
            assert!(inner.clean_lru.is_empty());
            assert!(Arc::ptr_eq(&inner.nodes[&40].data, &next));
        }
        let current = NodeWriteback {
            bid: 40,
            node: next.clone(),
        };
        cache.inner.lock().complete(&current);
        assert!(!cache.is_dirty());
        assert_eq!(cache.inner.lock().clean_lru.front(), Some(&40));
        // Retiring a dirty node removes the obligation rather than persisting
        // obsolete contents into a block that can be reused.
        cache.stage(40, next);
        cache.remove(40);
        assert!(!cache.is_dirty());
        assert!(cache.inner.lock().nodes.is_empty());
    }
}
