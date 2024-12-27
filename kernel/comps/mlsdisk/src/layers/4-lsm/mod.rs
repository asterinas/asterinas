// SPDX-License-Identifier: MPL-2.0

//! The layer of transactional Lsm-Tree.
//!
//! This module provides the implementation for `TxLsmTree`.
//! `TxLsmTree` is similar to general-purpose LSM-Tree, supporting `put()`, `get()`, `get_range()`
//! key-value records, which are managed in MemTables and SSTables.
//!
//! `TxLsmTree` is transactional in the sense that
//! 1) it supports `sync()` that guarantees changes are persisted atomically and irreversibly,
//!    synchronized records and unsynchronized records can co-existed.
//! 2) its internal data is securely stored in `TxLogStore` (L3) and updated in transactions for consistency,
//!    WALs and SSTables are stored and managed in `TxLogStore`.
//!
//! `TxLsmTree` supports piggybacking callbacks during compaction and recovery.
//!
//! # Usage Example
//!
//! Create a `TxLsmTree` then put some records into it.
//!
//! ```
//! // Prepare an underlying disk (implement `BlockSet`) first
//! let nblocks = 1024;
//! let mem_disk = MemDisk::create(nblocks)?;
//!
//! // Prepare an underlying `TxLogStore` (L3) for storing WALs and SSTs
//! let tx_log_store = Arc::new(TxLogStore::format(mem_disk)?);
//!
//! // Create a `TxLsmTree` with the created `TxLogStore`
//! let tx_lsm_tree: TxLsmTree<BlockId, String, MemDisk> =
//!     TxLsmTree::format(tx_log_store, Arc::new(YourFactory), None)?;
//!
//! // Put some key-value records into the tree
//! for i in 0..10 {
//!     let k = i as BlockId;
//!     let v = i.to_string();
//!     tx_lsm_tree.put(k, v)?;
//! }
//!
//! // Issue a sync operation to the tree to ensure persistency
//! tx_lsm_tree.sync()?;
//!
//! // Use `get()` (or `get_range()`) to query the tree
//! let target_value = tx_lsm_tree.get(&5).unwrap();
//! // Check the previously put value
//! assert_eq(target_value, "5");
//!
//! // `TxLsmTree` supports user-defined per-TX callbacks
//! struct YourFactory;
//! struct YourListener;
//!
//! impl<K, V> TxEventListenerFactory<K, V> for YourFactory {
//!     // Support create per-TX (upon compaction or upon recovery) listener
//!     fn new_event_listener(&self, tx_type: TxType) -> Arc<dyn TxEventListener<K, V>> {
//!         Arc::new(YourListener::new(tx_type))
//!     }
//! }
//!
//! // Support defining callbacks when record is added or drop, or
//! // at some critical points during a TX
//! impl<K, V> TxEventListener<K, V> for YourListener {
//!     /* details omitted, see the API for more */
//! }
//! ```

mod compaction;
mod mem_table;
mod range_query_ctx;
mod sstable;
mod tx_lsm_tree;
mod wal;

pub use self::{
    range_query_ctx::RangeQueryCtx,
    tx_lsm_tree::{
        AsKV, LsmLevel, RecordKey, RecordValue, SyncId, SyncIdStore, TxEventListener,
        TxEventListenerFactory, TxLsmTree, TxType,
    },
};
