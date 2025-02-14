// SPDX-License-Identifier: MPL-2.0

//! Block allocation.
use alloc::vec;
use core::{
    mem::size_of,
    num::NonZeroUsize,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use ostd_pod::Pod;
use serde::{Deserialize, Serialize};

use super::mlsdisk::Hba;
use crate::{
    layers::{
        bio::{BlockSet, Buf, BufRef, BID_SIZE},
        log::{TxLog, TxLogStore},
    },
    os::{BTreeMap, Condvar, CvarMutex, Mutex},
    prelude::*,
    util::BitMap,
};

/// The bucket name of block validity table.
const BUCKET_BLOCK_VALIDITY_TABLE: &str = "BVT";
/// The bucket name of block alloc/dealloc log.
const BUCKET_BLOCK_ALLOC_LOG: &str = "BAL";

/// Block validity table. Global allocator for `MlsDisk`,
/// which manages validities of user data blocks.
pub(super) struct AllocTable {
    bitmap: Mutex<BitMap>,
    next_avail: AtomicUsize,
    nblocks: NonZeroUsize,
    is_dirty: AtomicBool,
    cvar: Condvar,
    num_free: CvarMutex<usize>,
}

/// Per-TX block allocator in `MlsDisk`, recording validities
/// of user data blocks within each TX. All metadata will be stored in
/// `TxLog`s of bucket `BAL` during TX for durability and recovery purpose.
pub(super) struct BlockAlloc<D> {
    alloc_table: Arc<AllocTable>, // Point to the global allocator
    diff_table: Mutex<BTreeMap<Hba, AllocDiff>>, // Per-TX diffs of block validity
    store: Arc<TxLogStore<D>>,    // Store for diff log from L3
    diff_log: Mutex<Option<Arc<TxLog<D>>>>, // Opened diff log (currently not in-use)
}

/// Incremental diff of block validity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
enum AllocDiff {
    Alloc = 3,
    Dealloc = 7,
    Invalid,
}
const DIFF_RECORD_SIZE: usize = size_of::<AllocDiff>() + size_of::<Hba>();

impl AllocTable {
    /// Create a new `AllocTable` given the total number of blocks.
    pub fn new(nblocks: NonZeroUsize) -> Self {
        Self {
            bitmap: Mutex::new(BitMap::repeat(true, nblocks.get())),
            next_avail: AtomicUsize::new(0),
            nblocks,
            is_dirty: AtomicBool::new(false),
            cvar: Condvar::new(),
            num_free: CvarMutex::new(nblocks.get()),
        }
    }

    /// Allocate a free slot for a new block, returns `None`
    /// if there are no free slots.
    pub fn alloc(&self) -> Option<Hba> {
        let mut bitmap = self.bitmap.lock();
        let next_avail = self.next_avail.load(Ordering::Acquire);

        let hba = if let Some(hba) = bitmap.first_one(next_avail) {
            hba
        } else {
            bitmap.first_one(0)?
        };
        bitmap.set(hba, false);

        self.next_avail.store(hba + 1, Ordering::Release);
        Some(hba as Hba)
    }

    /// Allocate multiple free slots for a bunch of new blocks, returns `None`
    /// if there are no free slots for all.
    pub fn alloc_batch(&self, count: NonZeroUsize) -> Result<Vec<Hba>> {
        let cnt = count.get();
        let mut num_free = self.num_free.lock().unwrap();
        while *num_free < cnt {
            // TODO: May not be woken, may require manual triggering of a compaction in L4
            num_free = self.cvar.wait(num_free).unwrap();
        }
        debug_assert!(*num_free >= cnt);

        let hbas = self.do_alloc_batch(count).unwrap();
        debug_assert_eq!(hbas.len(), cnt);

        *num_free -= cnt;
        let _ = self
            .is_dirty
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed);
        Ok(hbas)
    }

    fn do_alloc_batch(&self, count: NonZeroUsize) -> Option<Vec<Hba>> {
        let count = count.get();
        debug_assert!(count > 0);
        let mut bitmap = self.bitmap.lock();
        let mut next_avail = self.next_avail.load(Ordering::Acquire);

        if next_avail + count > self.nblocks.get() {
            next_avail = bitmap.first_one(0)?;
        }

        let hbas = if let Some(hbas) = bitmap.first_ones(next_avail, count) {
            hbas
        } else {
            next_avail = bitmap.first_one(0)?;
            bitmap.first_ones(next_avail, count)?
        };
        hbas.iter().for_each(|hba| bitmap.set(*hba, false));

        next_avail = hbas.last().unwrap() + 1;
        self.next_avail.store(next_avail, Ordering::Release);
        Some(hbas)
    }

    /// Recover the `AllocTable` from the latest `BVT` log and a bunch of `BAL` logs
    /// in the given store.
    pub fn recover<D: BlockSet + 'static>(
        nblocks: NonZeroUsize,
        store: &Arc<TxLogStore<D>>,
    ) -> Result<Self> {
        let tx = store.new_tx();
        let res: Result<_> = tx.context(|| {
            // Recover the block validity table from `BVT` log first
            let bvt_log_res = store.open_log_in(BUCKET_BLOCK_VALIDITY_TABLE);
            let mut bitmap = match bvt_log_res {
                Ok(bvt_log) => {
                    let mut buf = Buf::alloc(bvt_log.nblocks())?;
                    bvt_log.read(0 as BlockId, buf.as_mut())?;
                    postcard::from_bytes(buf.as_slice()).map_err(|_| {
                        Error::with_msg(InvalidArgs, "deserialize block validity table failed")
                    })?
                }
                Err(e) => {
                    if e.errno() != NotFound {
                        return Err(e);
                    }
                    BitMap::repeat(true, nblocks.get())
                }
            };

            // Iterate each `BAL` log and apply each diff, from older to newer
            let bal_log_ids_res = store.list_logs_in(BUCKET_BLOCK_ALLOC_LOG);
            if let Err(e) = &bal_log_ids_res
                && e.errno() == NotFound
            {
                let next_avail = bitmap.first_one(0).unwrap_or(0);
                let num_free = bitmap.count_ones();
                return Ok(Self {
                    bitmap: Mutex::new(bitmap),
                    next_avail: AtomicUsize::new(next_avail),
                    nblocks,
                    is_dirty: AtomicBool::new(false),
                    cvar: Condvar::new(),
                    num_free: CvarMutex::new(num_free),
                });
            }
            let mut bal_log_ids = bal_log_ids_res?;
            bal_log_ids.sort();

            for bal_log_id in bal_log_ids {
                let bal_log_res = store.open_log(bal_log_id, false);
                if let Err(e) = &bal_log_res
                    && e.errno() == NotFound
                {
                    continue;
                }
                let bal_log = bal_log_res?;

                let log_nblocks = bal_log.nblocks();
                let mut buf = Buf::alloc(log_nblocks)?;
                bal_log.read(0 as BlockId, buf.as_mut())?;
                let buf_slice = buf.as_slice();
                let mut offset = 0;
                while offset <= log_nblocks * BLOCK_SIZE - DIFF_RECORD_SIZE {
                    let diff = AllocDiff::from(buf_slice[offset]);
                    offset += 1;
                    if diff == AllocDiff::Invalid {
                        continue;
                    }
                    let bid = BlockId::from_bytes(&buf_slice[offset..offset + BID_SIZE]);
                    offset += BID_SIZE;
                    match diff {
                        AllocDiff::Alloc => bitmap.set(bid, false),
                        AllocDiff::Dealloc => bitmap.set(bid, true),
                        _ => unreachable!(),
                    }
                }
            }
            let next_avail = bitmap.first_one(0).unwrap_or(0);
            let num_free = bitmap.count_ones();

            Ok(Self {
                bitmap: Mutex::new(bitmap),
                next_avail: AtomicUsize::new(next_avail),
                nblocks,
                is_dirty: AtomicBool::new(false),
                cvar: Condvar::new(),
                num_free: CvarMutex::new(num_free),
            })
        });
        let recov_self = res.map_err(|_| {
            tx.abort();
            Error::with_msg(TxAborted, "recover block validity table TX aborted")
        })?;
        tx.commit()?;

        Ok(recov_self)
    }

    /// Persist the block validity table to `BVT` log. GC all existed `BAL` logs.
    pub fn do_compaction<D: BlockSet + 'static>(&self, store: &Arc<TxLogStore<D>>) -> Result<()> {
        if !self.is_dirty.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Serialize the block validity table
        let bitmap = self.bitmap.lock();
        const BITMAP_MAX_SIZE: usize = 1792 * BLOCK_SIZE; // TBD
        let mut ser_buf = vec![0; BITMAP_MAX_SIZE];
        let ser_len = postcard::to_slice::<BitMap>(&bitmap, &mut ser_buf)
            .map_err(|_| Error::with_msg(InvalidArgs, "serialize block validity table failed"))?
            .len();
        ser_buf.resize(align_up(ser_len, BLOCK_SIZE), 0);
        drop(bitmap);

        // Persist the serialized block validity table to `BVT` log
        // and GC any old `BVT` logs and `BAL` logs
        let tx = store.new_tx();
        let res: Result<_> = tx.context(|| {
            if let Ok(bvt_log_ids) = store.list_logs_in(BUCKET_BLOCK_VALIDITY_TABLE) {
                for bvt_log_id in bvt_log_ids {
                    store.delete_log(bvt_log_id)?;
                }
            }

            let bvt_log = store.create_log(BUCKET_BLOCK_VALIDITY_TABLE)?;
            bvt_log.append(BufRef::try_from(&ser_buf[..]).unwrap())?;

            if let Ok(bal_log_ids) = store.list_logs_in(BUCKET_BLOCK_ALLOC_LOG) {
                for bal_log_id in bal_log_ids {
                    store.delete_log(bal_log_id)?;
                }
            }
            Ok(())
        });
        if res.is_err() {
            tx.abort();
            return_errno_with_msg!(TxAborted, "persist block validity table TX aborted");
        }
        tx.commit()?;

        self.is_dirty.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// Mark a specific slot deallocated.
    pub fn set_deallocated(&self, nth: usize) {
        let mut num_free = self.num_free.lock().unwrap();
        self.bitmap.lock().set(nth, true);

        *num_free += 1;
        const AVG_ALLOC_COUNT: usize = 1024;
        if *num_free >= AVG_ALLOC_COUNT {
            self.cvar.notify_one();
        }
    }
}

impl<D: BlockSet + 'static> BlockAlloc<D> {
    /// Create a new `BlockAlloc` with the given global allocator and store.
    pub fn new(alloc_table: Arc<AllocTable>, store: Arc<TxLogStore<D>>) -> Self {
        Self {
            alloc_table,
            diff_table: Mutex::new(BTreeMap::new()),
            store,
            diff_log: Mutex::new(None),
        }
    }

    /// Record a diff of `Alloc`.
    pub fn alloc_block(&self, block_id: Hba) -> Result<()> {
        let mut diff_table = self.diff_table.lock();
        let replaced = diff_table.insert(block_id, AllocDiff::Alloc);
        debug_assert!(
            replaced != Some(AllocDiff::Alloc),
            "can't allocate a block twice"
        );
        Ok(())
    }

    /// Record a diff of `Dealloc`.
    pub fn dealloc_block(&self, block_id: Hba) -> Result<()> {
        let mut diff_table = self.diff_table.lock();
        let replaced = diff_table.insert(block_id, AllocDiff::Dealloc);
        debug_assert!(
            replaced != Some(AllocDiff::Dealloc),
            "can't deallocate a block twice"
        );
        Ok(())
    }

    /// Prepare the block validity diff log.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn prepare_diff_log(&self) -> Result<()> {
        // Do nothing for now
        Ok(())
    }

    /// Persist the metadata in diff table to the block validity diff log.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn update_diff_log(&self) -> Result<()> {
        let diff_table = self.diff_table.lock();
        if diff_table.is_empty() {
            return Ok(());
        }

        let diff_log = self.store.create_log(BUCKET_BLOCK_ALLOC_LOG)?;

        const MAX_BUF_SIZE: usize = 1024 * BLOCK_SIZE;
        let mut diff_buf = Vec::with_capacity(MAX_BUF_SIZE);
        for (block_id, block_diff) in diff_table.iter() {
            diff_buf.push(*block_diff as u8);
            diff_buf.extend_from_slice(block_id.as_bytes());

            if diff_buf.len() + DIFF_RECORD_SIZE > MAX_BUF_SIZE {
                diff_buf.resize(align_up(diff_buf.len(), BLOCK_SIZE), 0);
                diff_log.append(BufRef::try_from(&diff_buf[..]).unwrap())?;
                diff_buf.clear();
            }
        }

        if diff_buf.is_empty() {
            return Ok(());
        }
        diff_buf.resize(align_up(diff_buf.len(), BLOCK_SIZE), 0);
        diff_log.append(BufRef::try_from(&diff_buf[..]).unwrap())
    }

    /// Update the metadata in diff table to the in-memory block validity table.
    pub fn update_alloc_table(&self) {
        let diff_table = self.diff_table.lock();
        let alloc_table = &self.alloc_table;
        let mut num_free = alloc_table.num_free.lock().unwrap();
        let mut bitmap = alloc_table.bitmap.lock();
        let mut num_dealloc = 0_usize;

        for (block_id, block_diff) in diff_table.iter() {
            match block_diff {
                AllocDiff::Alloc => {
                    debug_assert!(!bitmap[*block_id]);
                }
                AllocDiff::Dealloc => {
                    debug_assert!(!bitmap[*block_id]);
                    bitmap.set(*block_id, true);
                    num_dealloc += 1;
                }
                AllocDiff::Invalid => unreachable!(),
            };
        }

        *num_free += num_dealloc;
        const AVG_ALLOC_COUNT: usize = 1024;
        if *num_free >= AVG_ALLOC_COUNT {
            alloc_table.cvar.notify_one();
        }
    }
}

impl From<u8> for AllocDiff {
    fn from(value: u8) -> Self {
        match value {
            3 => AllocDiff::Alloc,
            7 => AllocDiff::Dealloc,
            _ => AllocDiff::Invalid,
        }
    }
}
