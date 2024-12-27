// SPDX-License-Identifier: MPL-2.0

//! Sorted String Table.
use alloc::vec;
use core::{marker::PhantomData, mem::size_of, num::NonZeroUsize, ops::RangeInclusive};

use lru::LruCache;
use ostd_pod::Pod;

use super::{
    mem_table::ValueEx, tx_lsm_tree::AsKVex, RangeQueryCtx, RecordKey, RecordValue, SyncId,
    TxEventListener,
};
use crate::{
    layers::{
        bio::{BlockSet, Buf, BufMut, BufRef, BID_SIZE},
        log::{TxLog, TxLogId, TxLogStore},
    },
    os::Mutex,
    prelude::*,
};

/// Sorted String Table (SST) for `TxLsmTree`.
///
/// Responsible for storing, managing key-value records on a `TxLog` (L3).
/// Records are serialized, sorted, organized on the `TxLog`.
/// Supports three access modes: point query, range query and whole scan.
pub(super) struct SSTable<K, V> {
    id: TxLogId,
    footer: Footer<K>,
    cache: Mutex<LruCache<BlockId, Arc<RecordBlock>>>,
    phantom: PhantomData<(K, V)>,
}

/// Footer of a `SSTable`, contains metadata of itself
/// index entries for locating record blocks.
#[derive(Debug)]
struct Footer<K> {
    meta: FooterMeta,
    index: Vec<IndexEntry<K>>,
}

/// Footer metadata to describe a `SSTable`.
#[repr(C)]
#[derive(Copy, Clone, Pod, Debug)]
struct FooterMeta {
    num_index: u16,
    index_nblocks: u16,
    total_records: u32,
    record_block_size: u32,
    sync_id: SyncId,
}
const FOOTER_META_SIZE: usize = size_of::<FooterMeta>();

/// Index entry to describe a `RecordBlock` in a `SSTable`.
#[derive(Debug)]
struct IndexEntry<K> {
    pos: BlockId,
    first: K,
    last: K,
}

/// A block full of serialized records.
struct RecordBlock {
    buf: Vec<u8>,
}
const RECORD_BLOCK_NBLOCKS: usize = 32;
/// The size of a `RecordBlock`, which is a multiple of `BLOCK_SIZE`.
const RECORD_BLOCK_SIZE: usize = RECORD_BLOCK_NBLOCKS * BLOCK_SIZE;

/// Accessor for a query.
enum QueryAccessor<K> {
    Point(K),
    Range(RangeInclusive<K>),
}

/// Iterator over `RecordBlock` for query purpose.
struct BlockQueryIter<'a, K, V> {
    block: &'a RecordBlock,
    offset: usize,
    accessor: &'a QueryAccessor<K>,
    phantom: PhantomData<(K, V)>,
}

/// Accessor for a whole table scan.
struct ScanAccessor<'a, K, V> {
    all_synced: bool,
    discard_unsynced: bool,
    event_listener: Option<&'a Arc<dyn TxEventListener<K, V>>>,
}

/// Iterator over `RecordBlock` for scan purpose.
struct BlockScanIter<'a, K, V> {
    block: Arc<RecordBlock>,
    offset: usize,
    accessor: ScanAccessor<'a, K, V>,
}

/// Iterator over `SSTable`.
pub(super) struct SstIter<'a, K, V, D> {
    sst: &'a SSTable<K, V>,
    curr_nth_index: usize,
    curr_rb_iter: Option<BlockScanIter<'a, K, V>>,
    tx_log_store: &'a Arc<TxLogStore<D>>,
}

/// Format on a `TxLog`:
///
/// ```text
/// |    [Record]     |    [Record]     |...|         Footer            |
/// |K|flag|V(V)| ... |    [Record]     |...| [IndexEntry] | FooterMeta |
/// |RECORD_BLOCK_SIZE|RECORD_BLOCK_SIZE|...|                           |
/// ```
impl<K: RecordKey<K>, V: RecordValue> SSTable<K, V> {
    const K_SIZE: usize = size_of::<K>();
    const V_SIZE: usize = size_of::<V>();
    const FLAG_SIZE: usize = size_of::<RecordFlag>();
    const MIN_RECORD_SIZE: usize = BID_SIZE + Self::FLAG_SIZE + Self::V_SIZE;
    const MAX_RECORD_SIZE: usize = BID_SIZE + Self::FLAG_SIZE + 2 * Self::V_SIZE;
    const INDEX_ENTRY_SIZE: usize = BID_SIZE + 2 * Self::K_SIZE;
    const CACHE_CAP: usize = 1024;

    /// Return the ID of this `SSTable`, which is the same ID
    /// to the underlying `TxLog`.
    pub fn id(&self) -> TxLogId {
        self.id
    }

    /// Return the sync ID of this `SSTable`, it may be smaller than the
    /// current master sync ID.
    pub fn sync_id(&self) -> SyncId {
        self.footer.meta.sync_id
    }

    /// The range of keys covered by this `SSTable`.
    pub fn range(&self) -> RangeInclusive<K> {
        RangeInclusive::new(
            self.footer.index[0].first,
            self.footer.index[self.footer.meta.num_index as usize - 1].last,
        )
    }

    /// Whether the target key is within the range, "within the range" doesn't mean
    /// the `SSTable` do have this key.
    pub fn is_within_range(&self, key: &K) -> bool {
        self.range().contains(key)
    }

    /// Whether the target range is overlapped with the range of this `SSTable`.
    pub fn overlap_with(&self, rhs_range: &RangeInclusive<K>) -> bool {
        let lhs_range = self.range();
        !(lhs_range.end() < rhs_range.start() || lhs_range.start() > rhs_range.end())
    }

    // Accessing functions below

    /// Point query.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn access_point<D: BlockSet + 'static>(
        &self,
        key: &K,
        tx_log_store: &Arc<TxLogStore<D>>,
    ) -> Result<V> {
        debug_assert!(self.range().contains(key));
        let target_rb_pos = self
            .footer
            .index
            .iter()
            .find_map(|entry| {
                if entry.is_within_range(key) {
                    Some(entry.pos)
                } else {
                    None
                }
            })
            .ok_or(Error::with_msg(NotFound, "target key not found in sst"))?;

        let accessor = QueryAccessor::Point(*key);
        let target_rb = self.target_record_block(target_rb_pos, tx_log_store)?;

        let mut iter = BlockQueryIter::<'_, K, V> {
            block: &target_rb,
            offset: 0,
            accessor: &accessor,
            phantom: PhantomData,
        };

        iter.find_map(|(k, v_opt)| if k == *key { v_opt } else { None })
            .ok_or(Error::with_msg(NotFound, "target value not found in SST"))
    }

    /// Range query.    
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn access_range<D: BlockSet + 'static>(
        &self,
        range_query_ctx: &mut RangeQueryCtx<K, V>,
        tx_log_store: &Arc<TxLogStore<D>>,
    ) -> Result<()> {
        debug_assert!(!range_query_ctx.is_completed());
        let range_uncompleted = range_query_ctx.range_uncompleted().unwrap();
        let target_rbs = self.footer.index.iter().filter_map(|entry| {
            if entry.overlap_with(&range_uncompleted) {
                Some(entry.pos)
            } else {
                None
            }
        });

        let accessor = QueryAccessor::Range(range_uncompleted.clone());
        for target_rb_pos in target_rbs {
            let target_rb = self.target_record_block(target_rb_pos, tx_log_store)?;

            let iter = BlockQueryIter::<'_, K, V> {
                block: &target_rb,
                offset: 0,
                accessor: &accessor,
                phantom: PhantomData,
            };

            let targets: Vec<_> = iter
                .filter_map(|(k, v_opt)| {
                    if range_uncompleted.contains(&k) {
                        Some((k, v_opt.unwrap()))
                    } else {
                        None
                    }
                })
                .collect();
            for (target_k, target_v) in targets {
                range_query_ctx.complete(target_k, target_v);
            }
        }
        Ok(())
    }

    /// Locate the target record block given its position, it
    /// resides in either the cache or the log.
    fn target_record_block<D: BlockSet + 'static>(
        &self,
        target_pos: BlockId,
        tx_log_store: &Arc<TxLogStore<D>>,
    ) -> Result<Arc<RecordBlock>> {
        let mut cache = self.cache.lock();
        if let Some(cached_rb) = cache.get(&target_pos) {
            Ok(cached_rb.clone())
        } else {
            let mut rb = RecordBlock::from_buf(vec![0; RECORD_BLOCK_SIZE]);
            // TODO: Avoid opening the log on every call
            let tx_log = tx_log_store.open_log(self.id, false)?;
            tx_log.read(target_pos, BufMut::try_from(rb.as_mut_slice()).unwrap())?;
            let rb = Arc::new(rb);
            cache.put(target_pos, rb.clone());
            Ok(rb)
        }
    }

    /// Return the iterator over this `SSTable`.
    /// The given `event_listener` (optional) is used on dropping records
    /// during iteration.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn iter<'a, D: BlockSet + 'static>(
        &'a self,
        sync_id: SyncId,
        discard_unsynced: bool,
        tx_log_store: &'a Arc<TxLogStore<D>>,
        event_listener: Option<&'a Arc<dyn TxEventListener<K, V>>>,
    ) -> SstIter<'a, K, V, D> {
        let all_synced = sync_id > self.sync_id();
        let accessor = ScanAccessor {
            all_synced,
            discard_unsynced,
            event_listener,
        };

        let first_rb = self
            .target_record_block(self.footer.index[0].pos, tx_log_store)
            .unwrap();

        SstIter {
            sst: self,
            curr_nth_index: 0,
            curr_rb_iter: Some(BlockScanIter {
                block: first_rb,
                offset: 0,
                accessor,
            }),
            tx_log_store,
        }
    }

    /// Scan the whole SST and collect all records.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn access_scan<D: BlockSet + 'static>(
        &self,
        sync_id: SyncId,
        discard_unsynced: bool,
        tx_log_store: &Arc<TxLogStore<D>>,
        event_listener: Option<&Arc<dyn TxEventListener<K, V>>>,
    ) -> Result<Vec<(K, ValueEx<V>)>> {
        let all_records = self
            .iter(sync_id, discard_unsynced, tx_log_store, event_listener)
            .collect();
        Ok(all_records)
    }

    // Building functions below

    /// Builds a SST given a bunch of records, after the SST becomes immutable.
    /// The given `event_listener` (optional) is used on adding records.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn build<'a, D: BlockSet + 'static, I, KVex>(
        records_iter: I,
        sync_id: SyncId,
        tx_log: &'a Arc<TxLog<D>>,
        event_listener: Option<&'a Arc<dyn TxEventListener<K, V>>>,
    ) -> Result<Self>
    where
        I: Iterator<Item = KVex>,
        KVex: AsKVex<K, V>,
        Self: 'a,
    {
        let mut cache = LruCache::new(NonZeroUsize::new(Self::CACHE_CAP).unwrap());
        let (total_records, index_vec) =
            Self::build_record_blocks(records_iter, tx_log, &mut cache, event_listener)?;
        let footer = Self::build_footer::<D>(index_vec, total_records, sync_id, tx_log)?;

        Ok(Self {
            id: tx_log.id(),
            footer,
            cache: Mutex::new(cache),
            phantom: PhantomData,
        })
    }

    /// Builds all the record blocks from the given records. Put the blocks to the log
    /// and the cache.
    fn build_record_blocks<'a, D: BlockSet + 'static, I, KVex>(
        records_iter: I,
        tx_log: &'a TxLog<D>,
        cache: &mut LruCache<BlockId, Arc<RecordBlock>>,
        event_listener: Option<&'a Arc<dyn TxEventListener<K, V>>>,
    ) -> Result<(usize, Vec<IndexEntry<K>>)>
    where
        I: Iterator<Item = KVex>,
        KVex: AsKVex<K, V>,
        Self: 'a,
    {
        let mut index_vec = Vec::new();
        let mut total_records = 0;
        let mut pos = 0 as BlockId;
        let (mut first_k, mut curr_k) = (None, None);
        let mut inner_offset = 0;

        let mut block_buf = Vec::with_capacity(RECORD_BLOCK_SIZE);
        for kv_ex in records_iter {
            let (key, value_ex) = (*kv_ex.key(), kv_ex.value_ex());
            total_records += 1;

            if inner_offset == 0 {
                debug_assert!(block_buf.is_empty());
                let _ = first_k.insert(key);
            }
            let _ = curr_k.insert(key);

            block_buf.extend_from_slice(key.as_bytes());
            inner_offset += Self::K_SIZE;

            match value_ex {
                ValueEx::Synced(v) => {
                    block_buf.push(RecordFlag::Synced as u8);
                    block_buf.extend_from_slice(v.as_bytes());

                    if let Some(listener) = event_listener {
                        listener.on_add_record(&(&key, v))?;
                    }
                    inner_offset += 1 + Self::V_SIZE;
                }
                ValueEx::Unsynced(v) => {
                    block_buf.push(RecordFlag::Unsynced as u8);
                    block_buf.extend_from_slice(v.as_bytes());

                    if let Some(listener) = event_listener {
                        listener.on_add_record(&(&key, v))?;
                    }
                    inner_offset += 1 + Self::V_SIZE;
                }
                ValueEx::SyncedAndUnsynced(sv, usv) => {
                    block_buf.push(RecordFlag::SyncedAndUnsynced as u8);
                    block_buf.extend_from_slice(sv.as_bytes());
                    block_buf.extend_from_slice(usv.as_bytes());

                    if let Some(listener) = event_listener {
                        listener.on_add_record(&(&key, sv))?;
                        listener.on_add_record(&(&key, usv))?;
                    }
                    inner_offset += Self::MAX_RECORD_SIZE;
                }
            }

            let cap_remained = RECORD_BLOCK_SIZE - inner_offset;
            if cap_remained >= Self::MAX_RECORD_SIZE {
                continue;
            }

            let index_entry = IndexEntry {
                pos,
                first: first_k.unwrap(),
                last: key,
            };
            build_one_record_block(&index_entry, &mut block_buf, tx_log, cache)?;
            index_vec.push(index_entry);

            pos += RECORD_BLOCK_NBLOCKS;
            inner_offset = 0;
            block_buf.clear();
        }
        debug_assert!(total_records > 0);

        if !block_buf.is_empty() {
            let last_entry = IndexEntry {
                pos,
                first: first_k.unwrap(),
                last: curr_k.unwrap(),
            };
            build_one_record_block(&last_entry, &mut block_buf, tx_log, cache)?;
            index_vec.push(last_entry);
        }

        fn build_one_record_block<K: RecordKey<K>, D: BlockSet + 'static>(
            entry: &IndexEntry<K>,
            buf: &mut Vec<u8>,
            tx_log: &TxLog<D>,
            cache: &mut LruCache<BlockId, Arc<RecordBlock>>,
        ) -> Result<()> {
            buf.resize(RECORD_BLOCK_SIZE, 0);
            let record_block = RecordBlock::from_buf(buf.clone());

            tx_log.append(BufRef::try_from(record_block.as_slice()).unwrap())?;
            cache.put(entry.pos, Arc::new(record_block));
            Ok(())
        }

        Ok((total_records, index_vec))
    }

    /// Builds the footer from the given index entries. The footer block will be appended
    /// to the SST log's end.
    fn build_footer<'a, D: BlockSet + 'static>(
        index_vec: Vec<IndexEntry<K>>,
        total_records: usize,
        sync_id: SyncId,
        tx_log: &'a TxLog<D>,
    ) -> Result<Footer<K>>
    where
        Self: 'a,
    {
        let footer_buf_len = align_up(
            index_vec.len() * Self::INDEX_ENTRY_SIZE + FOOTER_META_SIZE,
            BLOCK_SIZE,
        );
        let mut append_buf = Vec::with_capacity(footer_buf_len);
        for entry in &index_vec {
            append_buf.extend_from_slice(&entry.pos.to_le_bytes());
            append_buf.extend_from_slice(entry.first.as_bytes());
            append_buf.extend_from_slice(entry.last.as_bytes());
        }
        append_buf.resize(footer_buf_len, 0);
        let meta = FooterMeta {
            num_index: index_vec.len() as _,
            index_nblocks: (footer_buf_len / BLOCK_SIZE) as _,
            total_records: total_records as _,
            record_block_size: RECORD_BLOCK_SIZE as _,
            sync_id,
        };
        append_buf[footer_buf_len - FOOTER_META_SIZE..].copy_from_slice(meta.as_bytes());
        tx_log.append(BufRef::try_from(&append_buf[..]).unwrap())?;

        Ok(Footer {
            meta,
            index: index_vec,
        })
    }

    /// Builds a SST from a `TxLog`, loads the footer and the index blocks.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn from_log<D: BlockSet + 'static>(tx_log: &Arc<TxLog<D>>) -> Result<Self> {
        let nblocks = tx_log.nblocks();

        let mut rbuf = Buf::alloc(1)?;
        // Load footer block (last block)
        tx_log.read(nblocks - 1, rbuf.as_mut())?;
        let meta = FooterMeta::from_bytes(&rbuf.as_slice()[BLOCK_SIZE - FOOTER_META_SIZE..]);

        let mut rbuf = Buf::alloc(meta.index_nblocks as _)?;
        tx_log.read(nblocks - meta.index_nblocks as usize, rbuf.as_mut())?;
        let mut index = Vec::with_capacity(meta.num_index as _);
        let mut cache = LruCache::new(NonZeroUsize::new(Self::CACHE_CAP).unwrap());
        let mut record_block = vec![0; RECORD_BLOCK_SIZE];
        for i in 0..meta.num_index as _ {
            let buf =
                &rbuf.as_slice()[i * Self::INDEX_ENTRY_SIZE..(i + 1) * Self::INDEX_ENTRY_SIZE];

            let pos = BlockId::from_le_bytes(buf[..BID_SIZE].try_into().unwrap());
            let first = K::from_bytes(&buf[BID_SIZE..BID_SIZE + Self::K_SIZE]);
            let last =
                K::from_bytes(&buf[Self::INDEX_ENTRY_SIZE - Self::K_SIZE..Self::INDEX_ENTRY_SIZE]);

            tx_log.read(pos, BufMut::try_from(&mut record_block[..]).unwrap())?;
            let _ = cache.put(pos, Arc::new(RecordBlock::from_buf(record_block.clone())));

            index.push(IndexEntry { pos, first, last })
        }

        let footer = Footer { meta, index };
        Ok(Self {
            id: tx_log.id(),
            footer,
            cache: Mutex::new(cache),
            phantom: PhantomData,
        })
    }
}

impl<K: RecordKey<K>> IndexEntry<K> {
    pub fn range(&self) -> RangeInclusive<K> {
        self.first..=self.last
    }

    pub fn is_within_range(&self, key: &K) -> bool {
        self.range().contains(key)
    }

    pub fn overlap_with(&self, rhs_range: &RangeInclusive<K>) -> bool {
        let lhs_range = self.range();
        !(lhs_range.end() < rhs_range.start() || lhs_range.start() > rhs_range.end())
    }
}

impl RecordBlock {
    pub fn from_buf(buf: Vec<u8>) -> Self {
        debug_assert_eq!(buf.len(), RECORD_BLOCK_SIZE);
        Self { buf }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

impl<K: RecordKey<K>> QueryAccessor<K> {
    pub fn hit_target(&self, target: &K) -> bool {
        match self {
            QueryAccessor::Point(k) => k == target,
            QueryAccessor::Range(range) => range.contains(target),
        }
    }
}

impl<K: RecordKey<K>, V: RecordValue> Iterator for BlockQueryIter<'_, K, V> {
    type Item = (K, Option<V>);

    fn next(&mut self) -> Option<Self::Item> {
        let mut offset = self.offset;
        let buf_slice = &self.block.buf;
        let (k_size, v_size) = (SSTable::<K, V>::K_SIZE, SSTable::<K, V>::V_SIZE);

        if offset + SSTable::<K, V>::MIN_RECORD_SIZE > RECORD_BLOCK_SIZE {
            return None;
        }

        let key = K::from_bytes(&buf_slice[offset..offset + k_size]);
        offset += k_size;

        let flag = RecordFlag::from(buf_slice[offset]);
        offset += 1;
        if flag == RecordFlag::Invalid {
            return None;
        }

        let hit_target = self.accessor.hit_target(&key);
        let value_opt = match flag {
            RecordFlag::Synced | RecordFlag::Unsynced => {
                let v_opt = if hit_target {
                    Some(V::from_bytes(&buf_slice[offset..offset + v_size]))
                } else {
                    None
                };
                offset += v_size;
                v_opt
            }
            RecordFlag::SyncedAndUnsynced => {
                let v_opt = if hit_target {
                    Some(V::from_bytes(
                        &buf_slice[offset + v_size..offset + 2 * v_size],
                    ))
                } else {
                    None
                };
                offset += 2 * v_size;
                v_opt
            }
            _ => unreachable!(),
        };

        self.offset = offset;
        Some((key, value_opt))
    }
}

impl<K: RecordKey<K>, V: RecordValue> Iterator for BlockScanIter<'_, K, V> {
    type Item = (K, ValueEx<V>);

    fn next(&mut self) -> Option<Self::Item> {
        let mut offset = self.offset;
        let buf_slice = &self.block.buf;
        let (k_size, v_size) = (SSTable::<K, V>::K_SIZE, SSTable::<K, V>::V_SIZE);
        let (all_synced, discard_unsynced, event_listener) = (
            self.accessor.all_synced,
            self.accessor.discard_unsynced,
            &self.accessor.event_listener,
        );

        let (key, value_ex) = loop {
            if offset + SSTable::<K, V>::MIN_RECORD_SIZE > RECORD_BLOCK_SIZE {
                return None;
            }

            let key = K::from_bytes(&buf_slice[offset..offset + k_size]);
            offset += k_size;

            let flag = RecordFlag::from(buf_slice[offset]);
            offset += 1;
            if flag == RecordFlag::Invalid {
                return None;
            }

            let v_ex = match flag {
                RecordFlag::Synced => {
                    let v = V::from_bytes(&buf_slice[offset..offset + v_size]);
                    offset += v_size;
                    ValueEx::Synced(v)
                }
                RecordFlag::Unsynced => {
                    let v = V::from_bytes(&buf_slice[offset..offset + v_size]);
                    offset += v_size;
                    if all_synced {
                        ValueEx::Synced(v)
                    } else if discard_unsynced {
                        if let Some(listener) = event_listener {
                            listener.on_drop_record(&(key, v)).unwrap();
                        }
                        continue;
                    } else {
                        ValueEx::Unsynced(v)
                    }
                }
                RecordFlag::SyncedAndUnsynced => {
                    let sv = V::from_bytes(&buf_slice[offset..offset + v_size]);
                    offset += v_size;
                    let usv = V::from_bytes(&buf_slice[offset..offset + v_size]);
                    offset += v_size;
                    if all_synced {
                        if let Some(listener) = event_listener {
                            listener.on_drop_record(&(key, sv)).unwrap();
                        }
                        ValueEx::Synced(usv)
                    } else if discard_unsynced {
                        if let Some(listener) = event_listener {
                            listener.on_drop_record(&(key, usv)).unwrap();
                        }
                        ValueEx::Synced(sv)
                    } else {
                        ValueEx::SyncedAndUnsynced(sv, usv)
                    }
                }
                _ => unreachable!(),
            };
            break (key, v_ex);
        };

        self.offset = offset;
        Some((key, value_ex))
    }
}

impl<K: RecordKey<K>, V: RecordValue, D: BlockSet + 'static> Iterator for SstIter<'_, K, V, D> {
    type Item = (K, ValueEx<V>);

    fn next(&mut self) -> Option<Self::Item> {
        // Iterate over the current record block first
        if let Some(next) = self.curr_rb_iter.as_mut().unwrap().next() {
            return Some(next);
        }

        let curr_rb_iter = self.curr_rb_iter.take().unwrap();

        self.curr_nth_index += 1;
        // Iteration goes to the end
        if self.curr_nth_index >= self.sst.footer.meta.num_index as _ {
            return None;
        }

        // Ready to iterate the next record block
        let next_pos = self.sst.footer.index[self.curr_nth_index].pos;
        let next_rb = self
            .sst
            .target_record_block(next_pos, self.tx_log_store)
            .unwrap();

        let mut next_rb_iter = BlockScanIter {
            block: next_rb,
            offset: 0,
            accessor: curr_rb_iter.accessor,
        };
        let next = next_rb_iter.next()?;

        let _ = self.curr_rb_iter.insert(next_rb_iter);
        Some(next)
    }
}

impl<K: Debug, V> Debug for SSTable<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SSTable")
            .field("id", &self.id)
            .field("footer", &self.footer.meta)
            .field(
                "range",
                &RangeInclusive::new(
                    &self.footer.index[0].first,
                    &self.footer.index[self.footer.meta.num_index as usize - 1].last,
                ),
            )
            .finish()
    }
}

/// Flag bit for records in SSTable.
#[derive(PartialEq, Eq, Debug)]
#[repr(u8)]
enum RecordFlag {
    Synced = 7,
    Unsynced = 11,
    SyncedAndUnsynced = 19,
    Invalid,
}

impl From<u8> for RecordFlag {
    fn from(value: u8) -> Self {
        match value {
            7 => RecordFlag::Synced,
            11 => RecordFlag::Unsynced,
            19 => RecordFlag::SyncedAndUnsynced,
            _ => RecordFlag::Invalid,
        }
    }
}
