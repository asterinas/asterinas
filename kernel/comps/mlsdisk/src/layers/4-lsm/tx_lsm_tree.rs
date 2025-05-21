// SPDX-License-Identifier: MPL-2.0

//! Transactional LSM-Tree.
//!
//! API: `format()`, `recover()`, `get()`, `put()`, `get_range()`, `sync()`
//!
//! Responsible for managing two `MemTable`s, WAL and SSTs as `TxLog`s
//! backed by a `TxLogStore`. All operations are executed based
//! on internal transactions.
use alloc::vec;
use core::{
    hash::Hash,
    ops::{Add, RangeInclusive, Sub},
    sync::atomic::{AtomicU64, Ordering},
};

use ostd_pod::Pod;

use super::{
    compaction::Compactor,
    mem_table::{MemTableManager, ValueEx},
    range_query_ctx::RangeQueryCtx,
    sstable::SSTable,
    wal::{WalAppendTx, BUCKET_WAL},
};
use crate::{
    layers::{
        bio::BlockSet,
        log::{TxLogId, TxLogStore},
    },
    os::{spawn, BTreeMap, RwLock},
    prelude::*,
    tx::CurrentTx,
};

/// Monotonic incrementing sync ID.
pub type SyncId = u64;

/// A transactional LSM-Tree, managing `MemTable`s, WALs and SSTs backed by `TxLogStore` (L3).
///
/// Supports inserting and querying key-value records within transactions.
/// Supports user-defined callbacks in `MemTable`, during compaction and recovery.
pub struct TxLsmTree<K: RecordKey<K>, V, D>(Arc<TreeInner<K, V, D>>);

/// Inner structures of `TxLsmTree`.
pub(super) struct TreeInner<K: RecordKey<K>, V, D> {
    memtable_manager: MemTableManager<K, V>,
    sst_manager: RwLock<SstManager<K, V>>,
    wal_append_tx: WalAppendTx<D>,
    compactor: Compactor<K, V>,
    tx_log_store: Arc<TxLogStore<D>>,
    listener_factory: Arc<dyn TxEventListenerFactory<K, V>>,
    master_sync_id: MasterSyncId,
}

/// Levels in a `TxLsmTree`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum LsmLevel {
    L0 = 0,
    L1,
    L2,
    L3,
    L4,
    L5, // Cover over 10 TB data (over 800 TB user data from L5)
}

/// Manager of all `SSTable`s from every level in a `TxLsmTree`.
#[derive(Debug)]
struct SstManager<K, V> {
    level_ssts: Vec<BTreeMap<TxLogId, Arc<SSTable<K, V>>>>,
}

/// A factory of per-transaction event listeners.
pub trait TxEventListenerFactory<K, V>: Send + Sync {
    /// Creates a new event listener for a given transaction.
    fn new_event_listener(&self, tx_type: TxType) -> Arc<dyn TxEventListener<K, V>>;
}

/// An event listener that get informed when
/// 1) A new record is added,
/// 2) An existing record is dropped,
/// 3) After a TX began,
/// 4) Before a TX ended,
/// 5) After a TX committed.
///
/// `tx_type` indicates an internal transaction of `TxLsmTree`.
pub trait TxEventListener<K, V> {
    /// Notify the listener that a new record is added to a LSM-Tree.
    fn on_add_record(&self, record: &dyn AsKV<K, V>) -> Result<()>;

    /// Notify the listener that an existing record is dropped from a LSM-Tree.
    fn on_drop_record(&self, record: &dyn AsKV<K, V>) -> Result<()>;

    /// Notify the listener after a TX just began.
    fn on_tx_begin(&self, tx: &mut CurrentTx<'_>) -> Result<()>;

    /// Notify the listener before a tx ended.
    fn on_tx_precommit(&self, tx: &mut CurrentTx<'_>) -> Result<()>;

    /// Notify the listener after a TX committed.
    fn on_tx_commit(&self);
}

/// Types of `TxLsmTree`'s internal transactions.
#[derive(Copy, Clone, Debug)]
pub enum TxType {
    /// A Compaction Transaction merges old `SSTable`s into new ones.
    Compaction { to_level: LsmLevel },
    /// A Migration Transaction migrates synced records from old `SSTable`s
    /// new ones and discard unsynced records.
    Migration,
}

/// A trusted store that stores the master sync ID.
pub trait SyncIdStore: Send + Sync {
    /// Read the current master sync ID from the store.
    fn read(&self) -> Result<SyncId>;

    /// Write the given master sync ID to the store.
    fn write(&self, id: SyncId) -> Result<()>;
}

/// Master sync ID to help `TxLsmTree` achieve sync awareness.
pub(super) struct MasterSyncId {
    id: AtomicU64,
    store: Option<Arc<dyn SyncIdStore>>,
}

/// A trait that represents the key for a record in a `TxLsmTree`.
pub trait RecordKey<K>:
    Ord + Pod + Hash + Add<usize, Output = K> + Sub<K, Output = usize> + Debug + Send + Sync + 'static
{
}
/// A trait that represents the value for a record in a `TxLsmTree`.
pub trait RecordValue: Pod + Debug + Send + Sync + 'static {}

/// Represent any type that includes a key and a value.
pub trait AsKV<K, V>: Send + Sync {
    fn key(&self) -> &K;

    fn value(&self) -> &V;
}

/// A callback that will be called when drops a record.
pub type OnDropRecodeFn<K, V> = dyn Fn(&dyn AsKV<K, V>) + Send + Sync;

/// Represent any type that includes a key and a sync-aware value.
pub(super) trait AsKVex<K, V> {
    fn key(&self) -> &K;

    fn value_ex(&self) -> &ValueEx<V>;
}

/// Capacity of each `MemTable` and `SSTable`.
pub(super) const MEMTABLE_CAPACITY: usize = 2097152; // 96 MiB MemTable, cover 8 GiB data // TBD
pub(super) const SSTABLE_CAPACITY: usize = MEMTABLE_CAPACITY;

impl<K: RecordKey<K>, V: RecordValue, D: BlockSet + 'static> TxLsmTree<K, V, D> {
    /// Format a `TxLsmTree` from a given `TxLogStore`.
    pub fn format(
        tx_log_store: Arc<TxLogStore<D>>,
        listener_factory: Arc<dyn TxEventListenerFactory<K, V>>,
        on_drop_record_in_memtable: Option<Arc<OnDropRecodeFn<K, V>>>,
        sync_id_store: Option<Arc<dyn SyncIdStore>>,
    ) -> Result<Self> {
        let inner = TreeInner::format(
            tx_log_store,
            listener_factory,
            on_drop_record_in_memtable,
            sync_id_store,
        )?;
        Ok(Self(Arc::new(inner)))
    }

    /// Recover a `TxLsmTree` from a given `TxLogStore`.
    pub fn recover(
        tx_log_store: Arc<TxLogStore<D>>,
        listener_factory: Arc<dyn TxEventListenerFactory<K, V>>,
        on_drop_record_in_memtable: Option<Arc<OnDropRecodeFn<K, V>>>,
        sync_id_store: Option<Arc<dyn SyncIdStore>>,
    ) -> Result<Self> {
        let inner = TreeInner::recover(
            tx_log_store,
            listener_factory,
            on_drop_record_in_memtable,
            sync_id_store,
        )?;
        Ok(Self(Arc::new(inner)))
    }

    /// Gets a target value given a key.
    pub fn get(&self, key: &K) -> Result<V> {
        self.0.get(key)
    }

    /// Gets a range of target values given a range of keys.
    pub fn get_range(&self, range_query_ctx: &mut RangeQueryCtx<K, V>) -> Result<()> {
        self.0.get_range(range_query_ctx)
    }

    /// Puts a key-value record to the tree.
    pub fn put(&self, key: K, value: V) -> Result<()> {
        let inner = &self.0;
        let record = (key, value);

        // Write the record to WAL
        inner.wal_append_tx.append(&record)?;

        // Put the record into `MemTable`
        let at_capacity = inner.memtable_manager.put(key, value);
        if !at_capacity {
            return Ok(());
        }

        // Commit WAL TX before compaction
        // TODO: Error handling: try twice or ignore
        let wal_id = inner.wal_append_tx.commit()?;

        // Wait asynchronous compaction to finish
        // TODO: Error handling for compaction: try twice or become read-only
        inner.compactor.wait_compaction()?;

        inner.memtable_manager.switch().unwrap();

        // Trigger compaction when `MemTable` is at capacity
        self.do_compaction_tx(wal_id)
    }

    /// Persist all in-memory data of `TxLsmTree` to the backed storage.
    pub fn sync(&self) -> Result<()> {
        self.0.sync()
    }

    /// Do a compaction TX.
    /// The given `wal_id` is used to identify the WAL for discarding.
    fn do_compaction_tx(&self, wal_id: TxLogId) -> Result<()> {
        let inner = self.0.clone();
        let handle = spawn(move || -> Result<()> {
            // Do major compaction first if necessary
            if inner
                .sst_manager
                .read()
                .require_major_compaction(LsmLevel::L0)
            {
                inner.do_major_compaction(LsmLevel::L1)?;
            }

            // Do minor compaction
            inner.do_minor_compaction(wal_id)?;

            Ok(())
        });

        // handle.join().unwrap()?; // synchronous
        self.0.compactor.record_handle(handle); // asynchronous
        Ok(())
    }
}

impl<K: RecordKey<K>, V: RecordValue, D: BlockSet + 'static> TreeInner<K, V, D> {
    pub fn format(
        tx_log_store: Arc<TxLogStore<D>>,
        listener_factory: Arc<dyn TxEventListenerFactory<K, V>>,
        on_drop_record_in_memtable: Option<Arc<OnDropRecodeFn<K, V>>>,
        sync_id_store: Option<Arc<dyn SyncIdStore>>,
    ) -> Result<Self> {
        let sync_id: SyncId = 0;
        Ok(Self {
            memtable_manager: MemTableManager::new(
                sync_id,
                MEMTABLE_CAPACITY,
                on_drop_record_in_memtable,
            ),
            sst_manager: RwLock::new(SstManager::new()),
            wal_append_tx: WalAppendTx::new(&tx_log_store, sync_id),
            compactor: Compactor::new(),
            tx_log_store,
            listener_factory,
            master_sync_id: MasterSyncId::new(sync_id_store, sync_id)?,
        })
    }

    pub fn recover(
        tx_log_store: Arc<TxLogStore<D>>,
        listener_factory: Arc<dyn TxEventListenerFactory<K, V>>,
        on_drop_record_in_memtable: Option<Arc<OnDropRecodeFn<K, V>>>,
        sync_id_store: Option<Arc<dyn SyncIdStore>>,
    ) -> Result<Self> {
        let (synced_records, wal_sync_id) = Self::recover_from_wal(&tx_log_store)?;
        let (sst_manager, ssts_sync_id) = Self::recover_sst_manager(&tx_log_store)?;

        let max_sync_id = wal_sync_id.max(ssts_sync_id);
        let master_sync_id = MasterSyncId::new(sync_id_store, max_sync_id)?;
        let sync_id = master_sync_id.id();

        let memtable_manager = Self::recover_memtable_manager(
            sync_id,
            synced_records.into_iter(),
            on_drop_record_in_memtable,
        );

        let recov_self = Self {
            memtable_manager,
            sst_manager: RwLock::new(sst_manager),
            wal_append_tx: WalAppendTx::new(&tx_log_store, sync_id),
            compactor: Compactor::new(),
            tx_log_store,
            listener_factory,
            master_sync_id,
        };

        recov_self.do_migration_tx()?;

        debug!("[MlsDisk TxLsmTree] Recovery completed: {recov_self:?}");
        Ok(recov_self)
    }

    /// Recover the synced records and the maximum sync ID from the latest WAL.
    fn recover_from_wal(tx_log_store: &Arc<TxLogStore<D>>) -> Result<(Vec<(K, V)>, SyncId)> {
        let tx = tx_log_store.new_tx();
        let res: Result<_> = tx.context(|| {
            let wal_res = tx_log_store.open_log_in(BUCKET_WAL);
            if let Err(e) = &wal_res
                && e.errno() == NotFound
            {
                return Ok((vec![], 0));
            }
            let wal = wal_res?;
            // Only synced records count, all unsynced are discarded
            WalAppendTx::collect_synced_records_and_sync_id::<K, V>(&wal)
        });
        if res.is_ok() {
            tx.commit()?;
        } else {
            tx.abort();
            return_errno_with_msg!(TxAborted, "recover from WAL failed");
        }
        res
    }

    /// Recover `MemTable` from the given synced records.
    fn recover_memtable_manager(
        sync_id: SyncId,
        synced_records: impl Iterator<Item = (K, V)>,
        on_drop_record_in_memtable: Option<Arc<OnDropRecodeFn<K, V>>>,
    ) -> MemTableManager<K, V> {
        let memtable_manager =
            MemTableManager::new(sync_id, MEMTABLE_CAPACITY, on_drop_record_in_memtable);
        synced_records.into_iter().for_each(|(k, v)| {
            let _ = memtable_manager.put(k, v);
        });
        memtable_manager
    }

    /// Recover `SSTable`s from the given log store.
    /// Return the recovered `SstManager` and the maximum sync ID present.
    fn recover_sst_manager(
        tx_log_store: &Arc<TxLogStore<D>>,
    ) -> Result<(SstManager<K, V>, SyncId)> {
        let mut manager = SstManager::new();
        let mut max_sync_id: SyncId = 0;
        let tx = tx_log_store.new_tx();
        let res: Result<_> = tx.context(|| {
            for (level, bucket) in LsmLevel::iter() {
                let log_ids = tx_log_store.list_logs_in(bucket);
                if let Err(e) = &log_ids
                    && e.errno() == NotFound
                {
                    continue;
                }

                for id in log_ids? {
                    let log = tx_log_store.open_log(id, false)?;
                    let sst = SSTable::<K, V>::from_log(&log)?;
                    max_sync_id = max_sync_id.max(sst.sync_id());
                    manager.insert(SSTable::from_log(&log)?, level);
                }
            }
            Ok(())
        });
        if res.is_ok() {
            tx.commit()?;
        } else {
            tx.abort();
            return_errno_with_msg!(TxAborted, "recover TxLsmTree failed");
        }
        Ok((manager, max_sync_id))
    }

    pub fn get(&self, key: &K) -> Result<V> {
        // 1. Search from MemTables
        if let Some(value) = self.memtable_manager.get(key) {
            return Ok(value);
        }

        // 2. Search from SSTs (do Read TX)
        let value = self.do_read_tx(key)?;

        Ok(value)
    }

    pub fn get_range(&self, range_query_ctx: &mut RangeQueryCtx<K, V>) -> Result<()> {
        let is_completed = self.memtable_manager.get_range(range_query_ctx);
        if is_completed {
            return Ok(());
        }

        self.do_read_range_tx(range_query_ctx)?;

        Ok(())
    }

    pub fn sync(&self) -> Result<()> {
        let master_sync_id = self.master_sync_id.id() + 1;

        // Wait asynchronous compaction to finish
        // TODO: Error handling for compaction: try twice or become read-only
        self.compactor.wait_compaction()?;

        // TODO: Error handling for WAL: try twice or become read-only
        self.wal_append_tx.sync(master_sync_id)?;

        self.memtable_manager.sync(master_sync_id);

        // TODO: Error handling: try twice or ignore
        self.master_sync_id.increment()?;
        Ok(())
    }

    /// Read TX.
    fn do_read_tx(&self, key: &K) -> Result<V> {
        let tx = self.tx_log_store.new_tx();

        let read_res: Result<_> = tx.context(|| {
            // Search each level from top to bottom (newer to older)
            let sst_manager = self.sst_manager.read();

            for (level, _bucket) in LsmLevel::iter() {
                for (_id, sst) in sst_manager.list_level(level) {
                    if !sst.is_within_range(key) {
                        continue;
                    }

                    if let Ok(target_value) = sst.access_point(key, &self.tx_log_store) {
                        return Ok(target_value);
                    }
                }
            }

            return_errno_with_msg!(NotFound, "target sst not found");
        });
        if read_res.as_ref().is_err_and(|e| e.errno() != NotFound) {
            tx.abort();
            return_errno_with_msg!(TxAborted, "read TX failed")
        }

        tx.commit()?;

        read_res
    }

    /// Read Range TX.
    fn do_read_range_tx(&self, range_query_ctx: &mut RangeQueryCtx<K, V>) -> Result<()> {
        debug_assert!(!range_query_ctx.is_completed());
        let tx = self.tx_log_store.new_tx();

        let read_res: Result<_> = tx.context(|| {
            // Search each level from top to bottom (newer to older)
            let sst_manager = self.sst_manager.read();
            for (level, _bucket) in LsmLevel::iter() {
                for (_id, sst) in sst_manager.list_level(level) {
                    if !sst.overlap_with(&range_query_ctx.range_uncompleted().unwrap()) {
                        continue;
                    }

                    sst.access_range(range_query_ctx, &self.tx_log_store)?;

                    if range_query_ctx.is_completed() {
                        return Ok(());
                    }
                }
            }

            return_errno_with_msg!(NotFound, "target sst not found");
        });
        if read_res.as_ref().is_err_and(|e| e.errno() != NotFound) {
            tx.abort();
            return_errno_with_msg!(TxAborted, "read TX failed")
        }

        tx.commit()?;

        read_res
    }

    /// Minor Compaction TX { to_level: LsmLevel::L0 }.
    fn do_minor_compaction(&self, wal_id: TxLogId) -> Result<()> {
        let mut tx = self.tx_log_store.new_tx();
        // Prepare TX listener
        let tx_type = TxType::Compaction {
            to_level: LsmLevel::L0,
        };
        let event_listener = self.listener_factory.new_event_listener(tx_type);
        event_listener.on_tx_begin(&mut tx).map_err(|_| {
            tx.abort();
            Error::with_msg(
                TxAborted,
                "minor compaction TX callback 'on_tx_begin' failed",
            )
        })?;

        let res: Result<_> = tx.context(|| {
            let tx_log = self.tx_log_store.create_log(LsmLevel::L0.bucket())?;

            // Cook records in immutable MemTable into a new SST
            let immutable_memtable = self.memtable_manager.immutable_memtable();
            let records_iter = immutable_memtable.iter();
            let sync_id = immutable_memtable.sync_id();

            let sst = SSTable::build(records_iter, sync_id, &tx_log, Some(&event_listener))?;
            self.tx_log_store.delete_log(wal_id)?;
            Ok(sst)
        });
        let new_sst = res.map_err(|_| {
            tx.abort();
            Error::with_msg(TxAborted, "minor compaction TX failed")
        })?;

        event_listener.on_tx_precommit(&mut tx).map_err(|_| {
            tx.abort();
            Error::with_msg(
                TxAborted,
                "minor compaction TX callback 'on_tx_precommit' failed",
            )
        })?;

        tx.commit()?;
        event_listener.on_tx_commit();

        self.sst_manager.write().insert(new_sst, LsmLevel::L0);

        debug!("[MlsDisk TxLsmTree] Minor Compaction completed: {self:?}");
        Ok(())
    }

    /// Major Compaction TX { to_level: LsmLevel::L1~LsmLevel::L5 }.
    fn do_major_compaction(&self, to_level: LsmLevel) -> Result<()> {
        let from_level = to_level.upper_level();
        let mut tx = self.tx_log_store.new_tx();

        // Prepare TX listener
        let tx_type = TxType::Compaction { to_level };
        let event_listener = self.listener_factory.new_event_listener(tx_type);
        event_listener.on_tx_begin(&mut tx).map_err(|_| {
            tx.abort();
            Error::with_msg(
                TxAborted,
                "major compaction TX callback 'on_tx_begin' failed",
            )
        })?;

        let master_sync_id = self.master_sync_id.id();
        let tx_log_store = self.tx_log_store.clone();
        let listener = event_listener.clone();
        let res: Result<_> = tx.context(move || {
            let (mut created_ssts, mut deleted_ssts) = (vec![], vec![]);

            // Collect overlapped SSTs
            let sst_manager = self.sst_manager.read();
            let (upper_sst_id, upper_sst) = sst_manager
                .list_level(from_level)
                .last() // Choose the oldest SST from upper level
                .map(|(id, sst)| (*id, sst.clone()))
                .unwrap();
            let lower_ssts: Vec<(TxLogId, Arc<SSTable<K, V>>)> = {
                let mut ssts = sst_manager
                    .find_overlapped_ssts(&upper_sst.range(), to_level)
                    .map(|(id, sst)| (*id, sst.clone()))
                    .collect::<Vec<_>>();
                ssts.sort_by_key(|(_, sst)| *sst.range().start());
                ssts
            };
            drop(sst_manager);

            // If there are no overlapped SSTs, just move the upper SST to the lower level
            if lower_ssts.is_empty() {
                tx_log_store.move_log(upper_sst_id, from_level.bucket(), to_level.bucket())?;
                self.sst_manager
                    .write()
                    .move_sst(upper_sst_id, from_level, to_level);
                return Ok((created_ssts, deleted_ssts));
            }

            let upper_records_iter =
                upper_sst.iter(master_sync_id, false, &tx_log_store, Some(&listener));

            let lower_records_iter = lower_ssts.iter().flat_map(|(_, sst)| {
                sst.iter(master_sync_id, false, &tx_log_store, Some(&listener))
            });

            // Compact records then build new SSTs
            created_ssts = Compactor::compact_records_and_build_ssts(
                upper_records_iter,
                lower_records_iter,
                &tx_log_store,
                &listener,
                to_level,
                master_sync_id,
            )?;

            // Delete the old SSTs
            for (id, level) in core::iter::once((upper_sst_id, from_level))
                .chain(lower_ssts.into_iter().map(|(id, _)| (id, to_level)))
            {
                tx_log_store.delete_log(id)?;
                deleted_ssts.push((id, level));
            }
            Ok((created_ssts, deleted_ssts))
        });
        let (created_ssts, deleted_ssts) = res.map_err(|_| {
            tx.abort();
            Error::with_msg(TxAborted, "major compaction TX failed")
        })?;

        event_listener.on_tx_precommit(&mut tx).map_err(|_| {
            tx.abort();
            Error::with_msg(
                TxAborted,
                "major compaction TX callback 'on_tx_precommit' failed",
            )
        })?;

        tx.commit()?;
        event_listener.on_tx_commit();

        self.update_sst_manager(
            created_ssts.into_iter().map(|sst| (sst, to_level)),
            deleted_ssts.into_iter(),
        );

        debug!("[MlsDisk TxLsmTree] Major Compaction completed: {self:?}");

        // Continue to do major compaction if necessary
        if self.sst_manager.read().require_major_compaction(to_level) {
            self.do_major_compaction(to_level.lower_level())?;
        }
        Ok(())
    }

    /// Migration TX, primarily to discard all unsynced records in SSTs.
    fn do_migration_tx(&self) -> Result<()> {
        let mut tx = self.tx_log_store.new_tx();

        // Prepare TX listener
        let tx_type = TxType::Migration;
        let event_listener = self.listener_factory.new_event_listener(tx_type);
        event_listener.on_tx_begin(&mut tx).map_err(|_| {
            tx.abort();
            Error::with_msg(TxAborted, "migration TX callback 'on_tx_begin' failed")
        })?;

        let master_sync_id = self.master_sync_id.id();
        let tx_log_store = self.tx_log_store.clone();
        let listener = event_listener.clone();
        let res: Result<_> = tx.context(move || {
            let (mut created_ssts, mut deleted_ssts) = (vec![], vec![]);

            let sst_manager = self.sst_manager.read();
            for (level, bucket) in LsmLevel::iter() {
                let ssts = sst_manager.list_level(level);
                // Iterate SSTs whose sync ID is equal to the
                // master sync ID, who may have unsynced records
                for (&id, sst) in ssts.filter(|(_, sst)| sst.sync_id() == master_sync_id) {
                    // Collect synced records only
                    let mut synced_records_iter = sst
                        .iter(master_sync_id, true, &tx_log_store, Some(&listener))
                        .peekable();

                    if synced_records_iter.peek().is_some() {
                        // Create new migrated SST
                        let new_log = tx_log_store.create_log(bucket)?;
                        let new_sst =
                            SSTable::build(synced_records_iter, master_sync_id, &new_log, None)?;
                        created_ssts.push((new_sst, level));
                        continue;
                    }

                    // Delete the old SST
                    tx_log_store.delete_log(id)?;
                    deleted_ssts.push((id, level));
                }
            }

            Ok((created_ssts, deleted_ssts))
        });
        let (created_ssts, deleted_ssts) = res.map_err(|_| {
            tx.abort();
            Error::with_msg(TxAborted, "migration TX failed")
        })?;

        event_listener.on_tx_precommit(&mut tx).map_err(|_| {
            tx.abort();
            Error::with_msg(TxAborted, "migration TX callback 'on_tx_precommit' failed")
        })?;

        tx.commit()?;
        event_listener.on_tx_commit();

        self.update_sst_manager(created_ssts.into_iter(), deleted_ssts.into_iter());
        Ok(())
    }

    fn update_sst_manager(
        &self,
        created: impl Iterator<Item = (SSTable<K, V>, LsmLevel)>,
        deleted: impl Iterator<Item = (TxLogId, LsmLevel)>,
    ) {
        let mut sst_manager = self.sst_manager.write();
        created.for_each(|(sst, level)| {
            let _ = sst_manager.insert(sst, level);
        });
        deleted.for_each(|(id, level)| {
            let _ = sst_manager.remove(id, level);
        });
    }
}

impl MasterSyncId {
    /// Create a new instance of `MasterSyncId`.
    /// Load the master sync ID from the given store if present.
    /// If the store is not present, use the default sync ID instead.
    pub fn new(store: Option<Arc<dyn SyncIdStore>>, default: SyncId) -> Result<Self> {
        let id: SyncId = if let Some(store) = &store {
            store.read()?
        } else {
            default
        };
        Ok(Self {
            id: AtomicU64::new(id),
            store,
        })
    }

    /// Get the current master sync ID.
    pub fn id(&self) -> SyncId {
        self.id.load(Ordering::Acquire)
    }

    /// Increment the current master sync ID,
    /// store the new ID to the store if present.
    ///
    /// On success, return the new master sync ID.
    pub(super) fn increment(&self) -> Result<SyncId> {
        let incremented_id = self.id.fetch_add(1, Ordering::Release) + 1;
        if let Some(store) = &self.store {
            store.write(incremented_id)?;
        }
        Ok(incremented_id)
    }
}

impl<K: RecordKey<K>, V, D> Drop for TreeInner<K, V, D> {
    fn drop(&mut self) {
        // TODO: Should we commit the WAL TX before dropping?
        // let _ = self.wal_append_tx.commit();
    }
}

impl<K: RecordKey<K>, V: RecordValue, D: BlockSet + 'static> Debug for TreeInner<K, V, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TxLsmTree")
            .field("memtable_manager", &self.memtable_manager)
            .field("sst_manager", &self.sst_manager.read())
            .field("tx_log_store", &self.tx_log_store)
            .finish()
    }
}

impl<K: RecordKey<K>, V: RecordValue, D: BlockSet + 'static> Debug for TxLsmTree<K, V, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl LsmLevel {
    const LEVEL0_RATIO: u16 = 4;
    const LEVELI_RATIO: u16 = 10;

    const MAX_NUM_LEVELS: usize = 6;
    const LEVEL_BUCKETS: [(LsmLevel, &'static str); Self::MAX_NUM_LEVELS] = [
        (LsmLevel::L0, LsmLevel::L0.bucket()),
        (LsmLevel::L1, LsmLevel::L1.bucket()),
        (LsmLevel::L2, LsmLevel::L2.bucket()),
        (LsmLevel::L3, LsmLevel::L3.bucket()),
        (LsmLevel::L4, LsmLevel::L4.bucket()),
        (LsmLevel::L5, LsmLevel::L5.bucket()),
    ];

    pub fn iter() -> impl Iterator<Item = (LsmLevel, &'static str)> {
        Self::LEVEL_BUCKETS.iter().cloned()
    }

    pub fn upper_level(&self) -> LsmLevel {
        debug_assert!(*self != LsmLevel::L0);
        LsmLevel::from(*self as u8 - 1)
    }

    pub fn lower_level(&self) -> LsmLevel {
        debug_assert!(*self != LsmLevel::L5);
        LsmLevel::from(*self as u8 + 1)
    }

    pub const fn bucket(&self) -> &str {
        match self {
            LsmLevel::L0 => "L0",
            LsmLevel::L1 => "L1",
            LsmLevel::L2 => "L2",
            LsmLevel::L3 => "L3",
            LsmLevel::L4 => "L4",
            LsmLevel::L5 => "L5",
        }
    }
}

impl From<u8> for LsmLevel {
    fn from(value: u8) -> Self {
        match value {
            0 => LsmLevel::L0,
            1 => LsmLevel::L1,
            2 => LsmLevel::L2,
            3 => LsmLevel::L3,
            4 => LsmLevel::L4,
            5 => LsmLevel::L5,
            _ => unreachable!(),
        }
    }
}

impl<K: RecordKey<K>, V: RecordValue> SstManager<K, V> {
    pub fn new() -> Self {
        let level_ssts = (0..LsmLevel::MAX_NUM_LEVELS)
            .map(|_| BTreeMap::new())
            .collect();
        Self { level_ssts }
    }

    /// List all SSTs of a given level from newer to older.
    pub fn list_level(
        &self,
        level: LsmLevel,
    ) -> impl Iterator<Item = (&TxLogId, &Arc<SSTable<K, V>>)> {
        self.level_ssts[level as usize].iter().rev()
    }

    pub fn insert(&mut self, sst: SSTable<K, V>, level: LsmLevel) -> Option<Arc<SSTable<K, V>>> {
        let nth_level = level as usize;
        debug_assert!(nth_level < self.level_ssts.len());
        let level_ssts = &mut self.level_ssts[nth_level];
        level_ssts.insert(sst.id(), Arc::new(sst))
    }

    pub fn remove(&mut self, id: TxLogId, level: LsmLevel) -> Option<Arc<SSTable<K, V>>> {
        let level_ssts = &mut self.level_ssts[level as usize];
        level_ssts.remove(&id)
    }

    pub fn move_sst(&mut self, id: TxLogId, from: LsmLevel, to: LsmLevel) {
        let moved = self.level_ssts[from as usize].remove(&id).unwrap();
        let _ = self.level_ssts[to as usize].insert(id, moved);
    }

    /// Find overlapping SSTs at a given level with a given range.
    pub fn find_overlapped_ssts<'a>(
        &'a self,
        range: &'a RangeInclusive<K>,
        level: LsmLevel,
    ) -> impl Iterator<Item = (&'a TxLogId, &'a Arc<SSTable<K, V>>)> {
        self.list_level(level)
            .filter(|(_, sst)| sst.overlap_with(range))
    }

    /// Check whether a major compaction is required from `from_level` to its lower level.
    pub fn require_major_compaction(&self, from_level: LsmLevel) -> bool {
        debug_assert!(from_level != LsmLevel::L5);

        if from_level == LsmLevel::L0 {
            return self.level_ssts[from_level as usize].len() >= LsmLevel::LEVEL0_RATIO as _;
        }
        self.level_ssts[from_level as usize].len()
            >= LsmLevel::LEVELI_RATIO.pow(from_level as _) as _
    }
}

impl<K: Send + Sync, V: Send + Sync> AsKV<K, V> for (K, V) {
    fn key(&self) -> &K {
        &self.0
    }
    fn value(&self) -> &V {
        &self.1
    }
}

impl<K: Send + Sync, V: Send + Sync> AsKV<K, V> for (&K, &V) {
    fn key(&self) -> &K {
        self.0
    }
    fn value(&self) -> &V {
        self.1
    }
}

impl<K: Send + Sync, V: Send + Sync> AsKVex<K, V> for (K, ValueEx<V>) {
    fn key(&self) -> &K {
        &self.0
    }
    fn value_ex(&self) -> &ValueEx<V> {
        &self.1
    }
}

impl<K: Send + Sync, V: Send + Sync> AsKVex<K, V> for (&K, &ValueEx<V>) {
    fn key(&self) -> &K {
        self.0
    }
    fn value_ex(&self) -> &ValueEx<V> {
        self.1
    }
}

#[cfg(test)]
mod tests {
    use super::{super::RangeQueryCtx, *};
    use crate::{
        layers::bio::{Buf, MemDisk},
        os::{AeadKey as Key, AeadMac as Mac},
    };

    struct Factory;
    struct Listener;

    impl<K, V> TxEventListenerFactory<K, V> for Factory {
        fn new_event_listener(&self, _tx_type: TxType) -> Arc<dyn TxEventListener<K, V>> {
            Arc::new(Listener)
        }
    }
    impl<K, V> TxEventListener<K, V> for Listener {
        fn on_add_record(&self, _record: &dyn AsKV<K, V>) -> Result<()> {
            Ok(())
        }
        fn on_drop_record(&self, _record: &dyn AsKV<K, V>) -> Result<()> {
            Ok(())
        }
        fn on_tx_begin(&self, _tx: &mut Tx) -> Result<()> {
            Ok(())
        }
        fn on_tx_precommit(&self, _tx: &mut Tx) -> Result<()> {
            Ok(())
        }
        fn on_tx_commit(&self) {}
    }

    #[repr(C)]
    #[derive(Copy, Clone, Pod, Debug)]
    struct Value {
        pub hba: BlockId,
        pub key: Key,
        pub mac: Mac,
    }

    impl RecordKey<BlockId> for BlockId {}
    impl RecordValue for Value {}

    #[test]
    fn tx_lsm_tree_fns() -> Result<()> {
        let nblocks = 102400;
        let mem_disk = MemDisk::create(nblocks)?;
        let tx_log_store = Arc::new(TxLogStore::format(mem_disk, Key::random())?);
        let tx_lsm_tree: TxLsmTree<BlockId, Value, MemDisk> =
            TxLsmTree::format(tx_log_store.clone(), Arc::new(Factory), None, None)?;

        // Put sufficient records which can trigger compaction before a sync command
        let cap = MEMTABLE_CAPACITY;
        let start = 0;
        for i in start..start + cap {
            let (k, v) = (
                i as BlockId,
                Value {
                    hba: i as BlockId,
                    key: Key::random(),
                    mac: Mac::random(),
                },
            );
            tx_lsm_tree.put(k, v)?;
        }
        let target_value = tx_lsm_tree.get(&5).unwrap();
        assert_eq!(target_value.hba, 5);

        tx_lsm_tree.sync()?;

        let target_value = tx_lsm_tree.get(&500).unwrap();
        assert_eq!(target_value.hba, 500);

        // Put sufficient records which can trigger compaction after a sync command
        let start = 500;
        for i in start..start + cap {
            let (k, v) = (
                i as BlockId,
                Value {
                    hba: (i * 2) as BlockId,
                    key: Key::random(),
                    mac: Mac::random(),
                },
            );
            tx_lsm_tree.put(k, v)?;
        }

        let target_value = tx_lsm_tree.get(&500).unwrap();
        assert_eq!(target_value.hba, 1000);
        let target_value = tx_lsm_tree.get(&25).unwrap();
        assert_eq!(target_value.hba, 25);

        // Recover the `TxLsmTree`, all unsynced records should be discarded
        drop(tx_lsm_tree);
        let tx_lsm_tree: TxLsmTree<BlockId, Value, MemDisk> =
            TxLsmTree::recover(tx_log_store.clone(), Arc::new(Factory), None, None)?;

        assert!(tx_lsm_tree.get(&(600 + cap)).is_err());

        let cnt = 16;
        let mut range_query_ctx = RangeQueryCtx::new(500, cnt);
        tx_lsm_tree.get_range(&mut range_query_ctx).unwrap();
        let res = range_query_ctx.into_results();
        assert_eq!(res[0].1.hba, 500);
        assert_eq!(res[cnt - 1].1.hba, 500 + cnt - 1);
        Ok(())
    }
}
