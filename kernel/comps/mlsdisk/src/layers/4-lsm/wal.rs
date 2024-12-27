// SPDX-License-Identifier: MPL-2.0

//! Transactions in WriteAhead Log.
use alloc::vec;
use core::{fmt::Debug, mem::size_of};

use ostd_pod::Pod;

use super::{AsKV, SyncId};
use crate::{
    layers::{
        bio::{BlockId, BlockSet, Buf, BufRef},
        log::{TxLog, TxLogId, TxLogStore},
    },
    os::Mutex,
    prelude::*,
    tx::CurrentTx,
};

/// The bucket name of WAL.
pub(super) const BUCKET_WAL: &str = "WAL";

/// WAL append TX in `TxLsmTree`.
///
/// A `WalAppendTx` is used to append records, sync and discard WALs.
/// A WAL is storing, managing key-value records which are going to
/// put in `MemTable`. It's space is backed by a `TxLog` (L3).
#[derive(Clone)]
pub(super) struct WalAppendTx<D> {
    inner: Arc<Mutex<WalTxInner<D>>>,
}

struct WalTxInner<D> {
    /// The appended WAL of ongoing Tx.
    appended_log: Option<Arc<TxLog<D>>>,
    /// Current log ID of WAL for later use.
    log_id: Option<TxLogId>,
    /// Store current sync ID as the first record of WAL.
    sync_id: SyncId,
    /// A buffer to cache appended records.
    record_buf: Vec<u8>,
    /// Store for WALs.
    tx_log_store: Arc<TxLogStore<D>>,
}

impl<D: BlockSet + 'static> WalAppendTx<D> {
    const BUF_CAP: usize = 1024 * BLOCK_SIZE;

    /// Prepare a new WAL TX.
    pub fn new(store: &Arc<TxLogStore<D>>, sync_id: SyncId) -> Self {
        Self {
            inner: Arc::new(Mutex::new(WalTxInner {
                appended_log: None,
                log_id: None,
                sync_id,
                record_buf: Vec::with_capacity(Self::BUF_CAP),
                tx_log_store: store.clone(),
            })),
        }
    }

    /// Append phase for an Append TX, mainly to append newly records to the WAL.
    pub fn append<K: Pod, V: Pod>(&self, record: &dyn AsKV<K, V>) -> Result<()> {
        let mut inner = self.inner.lock();
        if inner.appended_log.is_none() {
            inner.prepare()?;
        }

        {
            let record_buf = &mut inner.record_buf;
            record_buf.push(WalAppendFlag::Record as u8);
            record_buf.extend_from_slice(record.key().as_bytes());
            record_buf.extend_from_slice(record.value().as_bytes());
        }

        const MAX_RECORD_SIZE: usize = 49;
        if inner.record_buf.len() <= Self::BUF_CAP - MAX_RECORD_SIZE {
            return Ok(());
        }

        inner.align_record_buf();
        let wal_tx = inner.tx_log_store.current_tx();
        let wal_log = inner.appended_log.as_ref().unwrap();
        self.flush_buf(&inner.record_buf, &wal_tx, wal_log)?;
        inner.record_buf.clear();

        Ok(())
    }

    /// Commit phase for an Append TX, mainly to commit (or abort) the TX.
    /// After the committed WAL is sealed. Return the corresponding log ID.
    ///
    /// # Panics
    ///
    /// This method panics if current WAL's TX does not exist.
    pub fn commit(&self) -> Result<TxLogId> {
        let mut inner = self.inner.lock();
        let wal_log = inner
            .appended_log
            .take()
            .expect("current WAL TX must exist");
        let wal_id = inner.log_id.take().unwrap();
        debug_assert_eq!(wal_id, wal_log.id());

        if !inner.record_buf.is_empty() {
            inner.align_record_buf();
            let wal_tx = inner.tx_log_store.current_tx();
            self.flush_buf(&inner.record_buf, &wal_tx, &wal_log)?;
            inner.record_buf.clear();
        }

        drop(wal_log);
        inner.tx_log_store.current_tx().commit()?;
        Ok(wal_id)
    }

    /// Appends current sync ID to WAL then commit the TX to ensure WAL's persistency.
    /// Save the log ID for later appending.
    pub fn sync(&self, sync_id: SyncId) -> Result<()> {
        let mut inner = self.inner.lock();
        if inner.appended_log.is_none() {
            inner.prepare()?;
        }
        inner.record_buf.push(WalAppendFlag::Sync as u8);
        inner.record_buf.extend_from_slice(&sync_id.to_le_bytes());
        inner.sync_id = sync_id;

        inner.align_record_buf();
        let wal_log = inner.appended_log.take().unwrap();
        self.flush_buf(
            &inner.record_buf,
            &inner.tx_log_store.current_tx(),
            &wal_log,
        )?;
        inner.record_buf.clear();

        drop(wal_log);
        inner.tx_log_store.current_tx().commit()
    }

    /// Flushes the buffer to the backed log.
    fn flush_buf(
        &self,
        record_buf: &[u8],
        wal_tx: &CurrentTx<'_>,
        log: &Arc<TxLog<D>>,
    ) -> Result<()> {
        debug_assert!(!record_buf.is_empty() && record_buf.len() % BLOCK_SIZE == 0);
        let res = wal_tx.context(|| {
            let buf = BufRef::try_from(record_buf).unwrap();
            log.append(buf)
        });
        if res.is_err() {
            wal_tx.abort();
        }
        res
    }

    /// Collects the synced records only and the maximum sync ID in the WAL.
    pub fn collect_synced_records_and_sync_id<K: Pod, V: Pod>(
        wal: &TxLog<D>,
    ) -> Result<(Vec<(K, V)>, SyncId)> {
        let nblocks = wal.nblocks();
        let mut records = Vec::new();

        // TODO: Allocate separate buffers for large WAL
        let mut buf = Buf::alloc(nblocks)?;
        wal.read(0 as BlockId, buf.as_mut())?;
        let buf_slice = buf.as_slice();

        let k_size = size_of::<K>();
        let v_size = size_of::<V>();
        let total_bytes = nblocks * BLOCK_SIZE;
        let mut offset = 0;
        let (mut max_sync_id, mut synced_len) = (None, 0);
        loop {
            const MIN_RECORD_SIZE: usize = 9;
            if offset > total_bytes - MIN_RECORD_SIZE {
                break;
            }

            let flag = WalAppendFlag::try_from(buf_slice[offset]);
            offset += 1;
            if flag.is_err() {
                continue;
            }

            match flag.unwrap() {
                WalAppendFlag::Record => {
                    let record = {
                        let k = K::from_bytes(&buf_slice[offset..offset + k_size]);
                        let v =
                            V::from_bytes(&buf_slice[offset + k_size..offset + k_size + v_size]);
                        offset += k_size + v_size;
                        (k, v)
                    };

                    records.push(record);
                }
                WalAppendFlag::Sync => {
                    let sync_id = SyncId::from_le_bytes(
                        buf_slice[offset..offset + size_of::<SyncId>()]
                            .try_into()
                            .unwrap(),
                    );
                    offset += size_of::<SyncId>();

                    let _ = max_sync_id.insert(sync_id);
                    synced_len = records.len();
                }
            }
        }

        if let Some(max_sync_id) = max_sync_id {
            records.truncate(synced_len);
            Ok((records, max_sync_id))
        } else {
            Ok((vec![], 0))
        }
    }
}

impl<D: BlockSet + 'static> WalTxInner<D> {
    /// Prepare phase for an Append TX, mainly to create new TX and WAL.
    pub fn prepare(&mut self) -> Result<()> {
        debug_assert!(self.appended_log.is_none());
        let appended_log = {
            let store = &self.tx_log_store;
            let wal_tx = store.new_tx();
            let log_id_opt = self.log_id;
            let res = wal_tx.context(|| {
                if let Some(log_id) = log_id_opt {
                    store.open_log(log_id, true)
                } else {
                    store.create_log(BUCKET_WAL)
                }
            });
            if res.is_err() {
                wal_tx.abort();
            }
            let wal_log = res?;
            let _ = self.log_id.insert(wal_log.id());
            wal_log
        };
        let _ = self.appended_log.insert(appended_log);

        // Record the sync ID at the beginning of the WAL
        debug_assert!(self.record_buf.is_empty());
        self.record_buf.push(WalAppendFlag::Sync as u8);
        self.record_buf
            .extend_from_slice(&self.sync_id.to_le_bytes());
        Ok(())
    }

    fn align_record_buf(&mut self) {
        let aligned_len = align_up(self.record_buf.len(), BLOCK_SIZE);
        self.record_buf.resize(aligned_len, 0);
    }
}

/// Two content kinds in a WAL.
#[derive(PartialEq, Eq, Debug)]
#[repr(u8)]
enum WalAppendFlag {
    Record = 13,
    Sync = 23,
}

impl TryFrom<u8> for WalAppendFlag {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            13 => Ok(WalAppendFlag::Record),
            23 => Ok(WalAppendFlag::Sync),
            _ => Err(Error::new(InvalidArgs)),
        }
    }
}
