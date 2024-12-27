// SPDX-License-Identifier: MPL-2.0

//! Compaction in `TxLsmTree`.
use core::marker::PhantomData;

use super::{
    mem_table::ValueEx, sstable::SSTable, tx_lsm_tree::SSTABLE_CAPACITY, LsmLevel, RecordKey,
    RecordValue, SyncId, TxEventListener,
};
use crate::{
    layers::{bio::BlockSet, log::TxLogStore},
    os::{JoinHandle, Mutex},
    prelude::*,
};

/// A `Compactor` is currently used for asynchronous compaction
/// and specific compaction algorithm of `TxLsmTree`.
pub(super) struct Compactor<K, V> {
    handle: Mutex<Option<JoinHandle<Result<()>>>>,
    phantom: PhantomData<(K, V)>,
}

impl<K: RecordKey<K>, V: RecordValue> Compactor<K, V> {
    /// Create a new `Compactor` instance.
    pub fn new() -> Self {
        Self {
            handle: Mutex::new(None),
            phantom: PhantomData,
        }
    }

    /// Record current compaction thread handle.
    pub fn record_handle(&self, handle: JoinHandle<Result<()>>) {
        let mut handle_opt = self.handle.lock();
        assert!(handle_opt.is_none());
        let _ = handle_opt.insert(handle);
    }

    /// Wait until the compaction is finished.
    pub fn wait_compaction(&self) -> Result<()> {
        if let Some(handle) = self.handle.lock().take() {
            handle.join().unwrap()
        } else {
            Ok(())
        }
    }

    /// Core function for compacting overlapped records and building new SSTs.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn compact_records_and_build_ssts<D: BlockSet + 'static>(
        upper_records: impl Iterator<Item = (K, ValueEx<V>)>,
        lower_records: impl Iterator<Item = (K, ValueEx<V>)>,
        tx_log_store: &Arc<TxLogStore<D>>,
        event_listener: &Arc<dyn TxEventListener<K, V>>,
        to_level: LsmLevel,
        sync_id: SyncId,
    ) -> Result<Vec<SSTable<K, V>>> {
        let mut created_ssts = Vec::new();
        let mut upper_iter = upper_records.peekable();
        let mut lower_iter = lower_records.peekable();

        loop {
            let mut record_cnt = 0;
            let records_iter = core::iter::from_fn(|| {
                if record_cnt == SSTABLE_CAPACITY {
                    return None;
                }

                record_cnt += 1;
                match (upper_iter.peek(), lower_iter.peek()) {
                    (Some((upper_k, _)), Some((lower_k, _))) => match upper_k.cmp(lower_k) {
                        core::cmp::Ordering::Less => upper_iter.next(),
                        core::cmp::Ordering::Greater => lower_iter.next(),
                        core::cmp::Ordering::Equal => {
                            let (k, new_v_ex) = upper_iter.next().unwrap();
                            let (_, old_v_ex) = lower_iter.next().unwrap();
                            let (next_v_ex, dropped_v_opt) =
                                Self::compact_value_ex(new_v_ex, old_v_ex);

                            if let Some(dropped_v) = dropped_v_opt {
                                event_listener.on_drop_record(&(k, dropped_v)).unwrap();
                            }
                            Some((k, next_v_ex))
                        }
                    },
                    (Some(_), None) => upper_iter.next(),
                    (None, Some(_)) => lower_iter.next(),
                    (None, None) => None,
                }
            });
            let mut records_iter = records_iter.peekable();
            if records_iter.peek().is_none() {
                break;
            }

            let new_log = tx_log_store.create_log(to_level.bucket())?;
            let new_sst = SSTable::build(records_iter, sync_id, &new_log, None)?;

            created_ssts.push(new_sst);
        }

        Ok(created_ssts)
    }

    /// Compact two `ValueEx<V>`s with the same key, returning
    /// the compacted value and the dropped value if any.
    fn compact_value_ex(new: ValueEx<V>, old: ValueEx<V>) -> (ValueEx<V>, Option<V>) {
        match (new, old) {
            (ValueEx::Synced(new_v), ValueEx::Synced(old_v)) => {
                (ValueEx::Synced(new_v), Some(old_v))
            }
            (ValueEx::Unsynced(new_v), ValueEx::Synced(old_v)) => {
                (ValueEx::SyncedAndUnsynced(old_v, new_v), None)
            }
            (ValueEx::Unsynced(new_v), ValueEx::Unsynced(old_v)) => {
                (ValueEx::Unsynced(new_v), Some(old_v))
            }
            (ValueEx::Unsynced(new_v), ValueEx::SyncedAndUnsynced(old_sv, old_usv)) => {
                (ValueEx::SyncedAndUnsynced(old_sv, new_v), Some(old_usv))
            }
            (ValueEx::SyncedAndUnsynced(new_sv, new_usv), ValueEx::Synced(old_sv)) => {
                (ValueEx::SyncedAndUnsynced(new_sv, new_usv), Some(old_sv))
            }
            _ => {
                unreachable!()
            }
        }
    }
}
