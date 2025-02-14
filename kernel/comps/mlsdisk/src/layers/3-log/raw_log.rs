// SPDX-License-Identifier: MPL-2.0

//! A store of raw (untrusted) logs.
//!
//! `RawLogStore<D>` allows creating, deleting, reading and writing
//! `RawLog<D>`. Each raw log is uniquely identified by its ID (`RawLogId`).
//! Writing to a raw log is append only.
//!
//! `RawLogStore<D>` stores raw logs on a disk of `D: BlockSet`.
//! Internally, `RawLogStore<D>` manages the disk space with `ChunkAlloc`
//! so that the disk space can be allocated and deallocated in the units of
//! chunk. An allocated chunk belongs to exactly one raw log. And one raw log
//! may be backed by multiple chunks. The raw log is represented externally
//! as a `BlockLog`.
//!
//! # Examples
//!
//! Raw logs are manipulated and accessed within transactions.
//!
//! ```
//! fn concat_logs<D>(
//!     log_store: &RawLogStore<D>,
//!     log_ids: &[RawLogId]
//! ) -> Result<RawLogId> {
//!     let mut tx = log_store.new_tx();
//!     let res: Result<_> = tx.context(|| {
//!         let mut buf = Buf::alloc(1)?;
//!         let output_log = log_store.create_log()?;
//!         for log_id in log_ids {
//!             let input_log = log_store.open_log(log_id, false)?;
//!             let input_len = input_log.nblocks();
//!             let mut pos = 0 as BlockId;
//!             while pos < input_len {
//!                 input_log.read(pos, buf.as_mut())?;
//!                 output_log.append(buf.as_ref())?;
//!             }
//!         }
//!         Ok(output_log.id())
//!     });
//!     if res.is_ok() {
//!         tx.commit()?;
//!     } else {
//!         tx.abort();
//!     }
//!     res
//! }
//! ```
//!
//! If any error occurs (e.g., failures to open, read, or write a log) during
//! the transaction, then all prior changes to raw logs shall have no
//! effects. On the other hand, if the commit operation succeeds, then
//! all changes made in the transaction shall take effect as a whole.
//!
//! # Expected behaviors
//!
//! We provide detailed descriptions about the expected behaviors of raw log
//! APIs under transactions.
//!
//! 1. The local changes made (e.g., creations, deletions, writes) in a TX are
//!    immediately visible to the TX, but not other TX until the TX is committed.
//!    For example, a newly-created log within TX A is immediately usable within TX,
//!    but becomes visible to other TX only until A is committed.
//!    As another example, when a log is deleted within a TX, then the TX can no
//!    longer open the log. But other concurrent TX can still open the log.
//!
//! 2. If a TX is aborted, then all the local changes made in the TX will be
//!    discarded.
//!
//! 3. At any given time, a log can have at most one writer TX.
//!    A TX becomes the writer of a log when the log is opened with the write
//!    permission in the TX. And it stops being the writer TX of the log only when
//!    the TX is terminated (not when the log is closed within TX).
//!    This single-writer rule avoids potential conflicts between concurrent
//!    writing to the same log.
//!
//! 4. Log creation does not conflict with log deleation, read, or write as
//!    every newly-created log is assigned a unique ID automatically.
//!
//! 4. Deleting a log does not affect any opened instance of the log in the TX
//!    or other TX (similar to deleting a file in a UNIX-style system).
//!    It is only until the deleting TX is committed and the last
//!    instance of the log is closed shall the log be deleted and its disk space
//!    be freed.
//!
//! 5. The TX commitment will not fail due to conflicts between concurrent
//!    operations in different TX.
use core::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use super::chunk::{ChunkAlloc, ChunkId, CHUNK_NBLOCKS};
use crate::{
    layers::{
        bio::{BlockLog, BlockSet, BufMut, BufRef},
        edit::Edit,
    },
    os::{HashMap, HashSet, Mutex, MutexGuard},
    prelude::*,
    tx::{CurrentTx, TxData, TxProvider},
    util::LazyDelete,
};

/// The ID of a raw log.
pub type RawLogId = u64;

/// A store of raw logs.
pub struct RawLogStore<D> {
    state: Arc<Mutex<State>>,
    disk: D,
    chunk_alloc: ChunkAlloc, // Mapping: ChunkId * CHUNK_NBLOCKS = disk position (BlockId)
    tx_provider: Arc<TxProvider>,
    weak_self: Weak<Self>,
}

impl<D: BlockSet> RawLogStore<D> {
    /// Creates a new store of raw logs,
    /// given a chunk allocator and an untrusted disk.
    pub fn new(disk: D, tx_provider: Arc<TxProvider>, chunk_alloc: ChunkAlloc) -> Arc<Self> {
        Self::from_parts(RawLogStoreState::new(), disk, chunk_alloc, tx_provider)
    }

    /// Constructs a `RawLogStore` from its parts.
    pub(super) fn from_parts(
        state: RawLogStoreState,
        disk: D,
        chunk_alloc: ChunkAlloc,
        tx_provider: Arc<TxProvider>,
    ) -> Arc<Self> {
        let new_self = {
            // Prepare lazy deletes first from persistent state
            let lazy_deletes = {
                let mut delete_table = HashMap::new();
                for (&log_id, log_entry) in state.log_table.iter() {
                    Self::add_lazy_delete(log_id, log_entry, &chunk_alloc, &mut delete_table)
                }
                delete_table
            };

            Arc::new_cyclic(|weak_self| Self {
                state: Arc::new(Mutex::new(State::new(state, lazy_deletes))),
                disk,
                chunk_alloc,
                tx_provider,
                weak_self: weak_self.clone(),
            })
        };

        // TX data
        new_self
            .tx_provider
            .register_data_initializer(Box::new(RawLogStoreEdit::new));

        // Commit handler
        new_self.tx_provider.register_commit_handler({
            let state = new_self.state.clone();
            let chunk_alloc = new_self.chunk_alloc.clone();
            move |current: CurrentTx<'_>| {
                current.data_with(|edit: &RawLogStoreEdit| {
                    if edit.edit_table.is_empty() {
                        return;
                    }

                    let mut state = state.lock();
                    state.apply(edit);

                    Self::add_lazy_deletes_for_created_logs(&mut state, edit, &chunk_alloc);
                });
                let mut state = state.lock();
                Self::do_lazy_deletion(&mut state, &current);
            }
        });

        new_self
    }

    // Adds a lazy delete for the given log.
    fn add_lazy_delete(
        log_id: RawLogId,
        log_entry: &RawLogEntry,
        chunk_alloc: &ChunkAlloc,
        delete_table: &mut HashMap<u64, Arc<LazyDelete<RawLogEntry>>>,
    ) {
        let log_entry = log_entry.clone();
        let chunk_alloc = chunk_alloc.clone();
        delete_table.insert(
            log_id,
            Arc::new(LazyDelete::new(log_entry, move |entry| {
                chunk_alloc.dealloc_batch(entry.head.chunks.iter().cloned())
            })),
        );
    }

    fn add_lazy_deletes_for_created_logs(
        state: &mut State,
        edit: &RawLogStoreEdit,
        chunk_alloc: &ChunkAlloc,
    ) {
        for log_id in edit.iter_created_logs() {
            let log_entry_opt = state.persistent.find_log(log_id);
            if log_entry_opt.is_none() || state.lazy_deletes.contains_key(&log_id) {
                continue;
            }

            Self::add_lazy_delete(
                log_id,
                log_entry_opt.as_ref().unwrap(),
                chunk_alloc,
                &mut state.lazy_deletes,
            )
        }
    }

    // Do lazy deletions for the deleted logs in the current TX.
    fn do_lazy_deletion(state: &mut State, current_tx: &CurrentTx) {
        let deleted_logs = current_tx
            .data_with(|edit: &RawLogStoreEdit| edit.iter_deleted_logs().collect::<Vec<_>>());

        for log_id in deleted_logs {
            let Some(lazy_delete) = state.lazy_deletes.remove(&log_id) else {
                // Other concurrent TXs have deleted the same log
                continue;
            };
            LazyDelete::delete(&lazy_delete);
        }
    }

    /// Creates a new transaction for `RawLogStore`.
    pub fn new_tx(&self) -> CurrentTx<'_> {
        self.tx_provider.new_tx()
    }

    /// Syncs all the data managed by `RawLogStore` for persistence.
    pub fn sync(&self) -> Result<()> {
        // Do nothing, leave the disk sync to `TxLogStore`
        Ok(())
    }

    /// Creates a new raw log with a new log ID.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn create_log(&self) -> Result<RawLog<D>> {
        let mut state = self.state.lock();
        let new_log_id = state.alloc_log_id();
        state
            .add_to_write_set(new_log_id)
            .expect("created log can't appear in write set");

        let mut current_tx = self.tx_provider.current();
        current_tx.data_mut_with(|edit: &mut RawLogStoreEdit| {
            edit.create_log(new_log_id);
        });

        Ok(RawLog {
            log_id: new_log_id,
            log_entry: None,
            log_store: self.weak_self.upgrade().unwrap(),
            tx_provider: self.tx_provider.clone(),
            lazy_delete: None,
            append_pos: AtomicUsize::new(0),
            can_append: true,
        })
    }

    /// Opens the raw log of a given ID.
    ///
    /// For any log at any time, there can be at most one TX that opens the log
    /// in the appendable mode.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn open_log(&self, log_id: u64, can_append: bool) -> Result<RawLog<D>> {
        let mut state = self.state.lock();
        // Must check lazy deletes first in case there is concurrent deletion
        let lazy_delete = state
            .lazy_deletes
            .get(&log_id)
            .ok_or(Error::with_msg(NotFound, "raw log already been deleted"))?
            .clone();
        let mut current_tx = self.tx_provider.current();

        let log_entry_opt = state.persistent.find_log(log_id);
        // The log is already created by other TX
        if let Some(log_entry) = log_entry_opt.as_ref() {
            if can_append {
                // Prevent other TX from opening this log in the append mode
                state.add_to_write_set(log_id)?;

                // If the log is open in the append mode, edit must be prepared
                current_tx.data_mut_with(|edit: &mut RawLogStoreEdit| {
                    edit.open_log(log_id, log_entry);
                });
            }
        }
        // The log must has been created by this TX
        else {
            let is_log_created =
                current_tx.data_mut_with(|edit: &mut RawLogStoreEdit| edit.is_log_created(log_id));
            if !is_log_created {
                return_errno_with_msg!(NotFound, "raw log not found");
            }
        }

        let append_pos: BlockId = log_entry_opt
            .as_ref()
            .map(|entry| entry.head.num_blocks as _)
            .unwrap_or(0);
        Ok(RawLog {
            log_id,
            log_entry: log_entry_opt.map(|entry| Arc::new(Mutex::new(entry.clone()))),
            log_store: self.weak_self.upgrade().unwrap(),
            tx_provider: self.tx_provider.clone(),
            lazy_delete: Some(lazy_delete),
            append_pos: AtomicUsize::new(append_pos),
            can_append,
        })
    }

    /// Deletes the raw log of a given ID.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn delete_log(&self, log_id: RawLogId) -> Result<()> {
        let mut current_tx = self.tx_provider.current();

        // Free tail chunks
        let tail_chunks =
            current_tx.data_mut_with(|edit: &mut RawLogStoreEdit| edit.delete_log(log_id));
        if let Some(chunks) = tail_chunks {
            self.chunk_alloc.dealloc_batch(chunks.iter().cloned());
        }
        // Leave freeing head chunks to lazy delete

        self.state.lock().remove_from_write_set(log_id);
        Ok(())
    }
}

impl<D> Debug for RawLogStore<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock();
        f.debug_struct("RawLogStore")
            .field("persistent_log_table", &state.persistent.log_table)
            .field("next_free_log_id", &state.next_free_log_id)
            .field("write_set", &state.write_set)
            .field("chunk_alloc", &self.chunk_alloc)
            .finish()
    }
}

/// A raw (untrusted) log.
pub struct RawLog<D> {
    log_id: RawLogId,
    log_entry: Option<Arc<Mutex<RawLogEntry>>>,
    log_store: Arc<RawLogStore<D>>,
    tx_provider: Arc<TxProvider>,
    lazy_delete: Option<Arc<LazyDelete<RawLogEntry>>>,
    append_pos: AtomicUsize,
    can_append: bool,
}

/// A reference (handle) to a raw log.
struct RawLogRef<'a, D> {
    log_store: &'a RawLogStore<D>,
    log_head: Option<RawLogHeadRef<'a>>,
    log_tail: Option<RawLogTailRef<'a>>,
}

/// A head reference (handle) to a raw log.
struct RawLogHeadRef<'a> {
    entry: MutexGuard<'a, RawLogEntry>,
}

/// A tail reference (handle) to a raw log.
struct RawLogTailRef<'a> {
    log_id: RawLogId,
    current: CurrentTx<'a>,
}

impl<D: BlockSet> BlockLog for RawLog<D> {
    /// Reads one or multiple blocks at a specified position.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    fn read(&self, pos: BlockId, buf: BufMut) -> Result<()> {
        let log_ref = self.as_ref();
        log_ref.read(pos, buf)
    }

    /// Appends one or multiple blocks at the end.
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    fn append(&self, buf: BufRef) -> Result<BlockId> {
        if !self.can_append {
            return_errno_with_msg!(PermissionDenied, "raw log not in append mode");
        }

        let mut log_ref = self.as_ref();
        log_ref.append(buf)?;

        let nblocks = buf.nblocks();
        let pos = self.append_pos.fetch_add(nblocks, Ordering::Release);
        Ok(pos)
    }

    /// Ensures that blocks are persisted to the disk.
    fn flush(&self) -> Result<()> {
        // FIXME: Should we sync the disk here?
        self.log_store.disk.flush()?;
        Ok(())
    }

    /// Returns the number of blocks.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    fn nblocks(&self) -> usize {
        let log_ref = self.as_ref();
        log_ref.nblocks()
    }
}

impl<D> RawLog<D> {
    /// Gets the unique ID of raw log.
    pub fn id(&self) -> RawLogId {
        self.log_id
    }

    /// Gets the reference (handle) of raw log.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    fn as_ref(&self) -> RawLogRef<'_, D> {
        let log_head = self.log_entry.as_ref().map(|entry| RawLogHeadRef {
            entry: entry.lock(),
        });
        let log_tail = {
            // Check if the log exists create or append edit
            let has_valid_edit = self.tx_provider.current().data_mut_with(
                |store_edit: &mut RawLogStoreEdit| -> bool {
                    let Some(edit) = store_edit.edit_table.get(&self.log_id) else {
                        return false;
                    };
                    match edit {
                        RawLogEdit::Create(_) | RawLogEdit::Append(_) => true,
                        RawLogEdit::Delete => false,
                    }
                },
            );
            if has_valid_edit {
                Some(RawLogTailRef {
                    log_id: self.log_id,
                    current: self.tx_provider.current(),
                })
            } else {
                None
            }
        };

        RawLogRef {
            log_store: &self.log_store,
            log_head,
            log_tail,
        }
    }
}

impl<D> Drop for RawLog<D> {
    fn drop(&mut self) {
        if self.can_append {
            self.log_store
                .state
                .lock()
                .remove_from_write_set(self.log_id);
        }
    }
}

impl<D> Debug for RawLog<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawLog")
            .field("log_id", &self.log_id)
            .field("log_entry", &self.log_entry)
            .field("append_pos", &self.append_pos)
            .field("can_append", &self.can_append)
            .finish()
    }
}

impl<D: BlockSet> RawLogRef<'_, D> {
    /// Reads one or multiple blocks at a specified position of the log.
    /// First head then tail if necessary.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn read(&self, mut pos: BlockId, mut buf: BufMut) -> Result<()> {
        let mut nblocks = buf.nblocks();
        let mut buf_slice = buf.as_mut_slice();

        let head_len = self.head_len();
        let tail_len = self.tail_len();
        let total_len = head_len + tail_len;

        if pos + nblocks > total_len {
            return_errno_with_msg!(InvalidArgs, "do not allow short read");
        }

        let disk = &self.log_store.disk;
        // Read from the head if possible and necessary
        let head_opt = &self.log_head;
        if let Some(head) = head_opt
            && pos < head_len
        {
            let num_read = nblocks.min(head_len - pos);

            let read_buf = BufMut::try_from(&mut buf_slice[..num_read * BLOCK_SIZE])?;
            head.read(pos, read_buf, &disk)?;

            pos += num_read;
            nblocks -= num_read;
            buf_slice = &mut buf_slice[num_read * BLOCK_SIZE..];
        }
        if nblocks == 0 {
            return Ok(());
        }

        // Read from the tail if possible and necessary
        let tail_opt = &self.log_tail;
        if let Some(tail) = tail_opt
            && pos >= head_len
        {
            let num_read = nblocks.min(total_len - pos);
            let read_buf = BufMut::try_from(&mut buf_slice[..num_read * BLOCK_SIZE])?;

            tail.read(pos - head_len, read_buf, &disk)?;
        }
        Ok(())
    }

    /// Appends one or multiple blocks at the end (to the tail).
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn append(&mut self, buf: BufRef) -> Result<()> {
        let append_nblocks = buf.nblocks();
        let log_tail = self
            .log_tail
            .as_mut()
            .expect("raw log must be opened in append mode");

        // Allocate new chunks if necessary
        let new_chunks_opt = {
            let chunks_needed = log_tail.calc_needed_chunks(append_nblocks);
            if chunks_needed > 0 {
                let chunk_ids = self
                    .log_store
                    .chunk_alloc
                    .alloc_batch(chunks_needed)
                    .ok_or(Error::with_msg(OutOfMemory, "chunk allocation failed"))?;
                Some(chunk_ids)
            } else {
                None
            }
        };

        if let Some(new_chunks) = new_chunks_opt {
            log_tail.tail_mut_with(|tail: &mut RawLogTail| {
                tail.chunks.extend(new_chunks);
            });
        }

        log_tail.append(buf, &self.log_store.disk)?;

        // Update tail metadata
        log_tail.tail_mut_with(|tail: &mut RawLogTail| {
            tail.num_blocks += append_nblocks as u32;
        });
        Ok(())
    }

    /// Returns the number of blocks.
    ///
    /// # Panics
    ///
    /// This method must be called within a TX. Otherwise, this method panics.
    pub fn nblocks(&self) -> usize {
        self.head_len() + self.tail_len()
    }

    fn head_len(&self) -> usize {
        self.log_head.as_ref().map_or(0, |head| head.len())
    }

    fn tail_len(&self) -> usize {
        self.log_tail.as_ref().map_or(0, |tail| tail.len())
    }
}

impl RawLogHeadRef<'_> {
    pub fn len(&self) -> usize {
        self.entry.head.num_blocks as _
    }

    pub fn read<D: BlockSet>(&self, offset: BlockId, mut buf: BufMut, disk: &D) -> Result<()> {
        let nblocks = buf.nblocks();
        debug_assert!(offset + nblocks <= self.entry.head.num_blocks as _);

        let prepared_blocks = self.prepare_blocks(offset, nblocks);
        debug_assert_eq!(prepared_blocks.len(), nblocks);

        // Batch read
        // Note that `prepared_blocks` are not always sorted
        let mut offset = 0;
        for consecutive_blocks in prepared_blocks.chunk_by(|b1, b2| b2.saturating_sub(*b1) == 1) {
            let len = consecutive_blocks.len();
            let first_bid = *consecutive_blocks.first().unwrap();
            let buf_slice =
                &mut buf.as_mut_slice()[offset * BLOCK_SIZE..(offset + len) * BLOCK_SIZE];
            disk.read(first_bid, BufMut::try_from(buf_slice).unwrap())?;
            offset += len;
        }

        Ok(())
    }

    /// Collect and prepare a set of consecutive blocks in head for a read request.
    pub fn prepare_blocks(&self, mut offset: BlockId, nblocks: usize) -> Vec<BlockId> {
        let mut res_blocks = Vec::with_capacity(nblocks);
        let chunks = &self.entry.head.chunks;

        while res_blocks.len() != nblocks {
            let curr_chunk_idx = offset / CHUNK_NBLOCKS;
            let curr_chunk_inner_offset = offset % CHUNK_NBLOCKS;

            res_blocks.push(chunks[curr_chunk_idx] * CHUNK_NBLOCKS + curr_chunk_inner_offset);
            offset += 1;
        }

        res_blocks
    }
}

impl RawLogTailRef<'_> {
    /// Apply given function to the immutable tail.
    pub fn tail_with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&RawLogTail) -> R,
    {
        self.current.data_with(|store_edit: &RawLogStoreEdit| -> R {
            let edit = store_edit.edit_table.get(&self.log_id).unwrap();
            match edit {
                RawLogEdit::Create(create) => f(&create.tail),
                RawLogEdit::Append(append) => f(&append.tail),
                RawLogEdit::Delete => unreachable!(),
            }
        })
    }

    /// Apply given function to the mutable tail.
    pub fn tail_mut_with<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut RawLogTail) -> R,
    {
        self.current
            .data_mut_with(|store_edit: &mut RawLogStoreEdit| -> R {
                let edit = store_edit.edit_table.get_mut(&self.log_id).unwrap();
                match edit {
                    RawLogEdit::Create(create) => f(&mut create.tail),
                    RawLogEdit::Append(append) => f(&mut append.tail),
                    RawLogEdit::Delete => unreachable!(),
                }
            })
    }

    pub fn len(&self) -> usize {
        self.tail_with(|tail: &RawLogTail| tail.num_blocks as _)
    }

    pub fn read<D: BlockSet>(&self, offset: BlockId, mut buf: BufMut, disk: &D) -> Result<()> {
        let nblocks = buf.nblocks();
        let tail_nblocks = self.len();
        debug_assert!(offset + nblocks <= tail_nblocks);

        let prepared_blocks = self.prepare_blocks(offset, nblocks);
        debug_assert_eq!(prepared_blocks.len(), nblocks);

        // Batch read
        // Note that `prepared_blocks` are not always sorted
        let mut offset = 0;
        for consecutive_blocks in prepared_blocks.chunk_by(|b1, b2| b2.saturating_sub(*b1) == 1) {
            let len = consecutive_blocks.len();
            let first_bid = *consecutive_blocks.first().unwrap();
            let buf_slice =
                &mut buf.as_mut_slice()[offset * BLOCK_SIZE..(offset + len) * BLOCK_SIZE];
            disk.read(first_bid, BufMut::try_from(buf_slice).unwrap())?;
            offset += len;
        }

        Ok(())
    }

    pub fn append<D: BlockSet>(&self, buf: BufRef, disk: &D) -> Result<()> {
        let nblocks = buf.nblocks();

        let prepared_blocks = self.prepare_blocks(self.len() as _, nblocks);
        debug_assert_eq!(prepared_blocks.len(), nblocks);

        // Batch write
        // Note that `prepared_blocks` are not always sorted
        let mut offset = 0;
        for consecutive_blocks in prepared_blocks.chunk_by(|b1, b2| b2.saturating_sub(*b1) == 1) {
            let len = consecutive_blocks.len();
            let first_bid = *consecutive_blocks.first().unwrap();
            let buf_slice = &buf.as_slice()[offset * BLOCK_SIZE..(offset + len) * BLOCK_SIZE];
            disk.write(first_bid, BufRef::try_from(buf_slice).unwrap())?;
            offset += len;
        }

        Ok(())
    }

    // Calculate how many new chunks we need for an append request
    pub fn calc_needed_chunks(&self, append_nblocks: usize) -> usize {
        self.tail_with(|tail: &RawLogTail| {
            let avail_blocks = tail.head_last_chunk_free_blocks as usize
                + tail.chunks.len() * CHUNK_NBLOCKS
                - tail.num_blocks as usize;
            if append_nblocks > avail_blocks {
                align_up(append_nblocks - avail_blocks, CHUNK_NBLOCKS) / CHUNK_NBLOCKS
            } else {
                0
            }
        })
    }

    /// Collect and prepare a set of consecutive blocks in tail for a read/append request.
    fn prepare_blocks(&self, mut offset: BlockId, nblocks: usize) -> Vec<BlockId> {
        self.tail_with(|tail: &RawLogTail| {
            let mut res_blocks = Vec::with_capacity(nblocks);
            let head_last_chunk_free_blocks = tail.head_last_chunk_free_blocks as usize;

            // Collect available blocks from the last chunk of the head first if necessary
            if offset < head_last_chunk_free_blocks as _ {
                for i in offset..head_last_chunk_free_blocks {
                    let avail_chunk = tail.head_last_chunk_id * CHUNK_NBLOCKS
                        + (CHUNK_NBLOCKS - head_last_chunk_free_blocks + i);
                    res_blocks.push(avail_chunk);

                    if res_blocks.len() == nblocks {
                        return res_blocks;
                    }
                }

                offset = 0;
            } else {
                offset -= head_last_chunk_free_blocks;
            }

            // Collect available blocks from the tail first if necessary
            let chunks = &tail.chunks;
            while res_blocks.len() != nblocks {
                let curr_chunk_idx = offset / CHUNK_NBLOCKS;
                let curr_chunk_inner_offset = offset % CHUNK_NBLOCKS;

                res_blocks.push(chunks[curr_chunk_idx] * CHUNK_NBLOCKS + curr_chunk_inner_offset);
                offset += 1;
            }

            res_blocks
        })
    }
}

////////////////////////////////////////////////////////////////////////////////
// Persistent State
////////////////////////////////////////////////////////////////////////////////

/// The volatile and persistent state of a `RawLogStore`.
struct State {
    persistent: RawLogStoreState,
    next_free_log_id: u64,
    write_set: HashSet<RawLogId>,
    lazy_deletes: HashMap<RawLogId, Arc<LazyDelete<RawLogEntry>>>,
}

/// The persistent state of a `RawLogStore`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawLogStoreState {
    log_table: HashMap<RawLogId, RawLogEntry>,
}

/// A log entry implies the persistent state of the raw log.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RawLogEntry {
    head: RawLogHead,
}

/// A log head contains chunk metadata of a log's already-persist data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RawLogHead {
    pub chunks: Vec<ChunkId>,
    pub num_blocks: u32,
}

impl State {
    pub fn new(
        persistent: RawLogStoreState,
        lazy_deletes: HashMap<RawLogId, Arc<LazyDelete<RawLogEntry>>>,
    ) -> Self {
        let next_free_log_id = if let Some(max_log_id) = lazy_deletes.keys().max() {
            max_log_id + 1
        } else {
            0
        };
        Self {
            persistent: persistent.clone(),
            next_free_log_id,
            write_set: HashSet::new(),
            lazy_deletes,
        }
    }

    pub fn apply(&mut self, edit: &RawLogStoreEdit) {
        edit.apply_to(&mut self.persistent);
    }

    pub fn alloc_log_id(&mut self) -> u64 {
        let new_log_id = self.next_free_log_id;
        self.next_free_log_id = self
            .next_free_log_id
            .checked_add(1)
            .expect("64-bit IDs won't be exhausted even though IDs are not recycled");
        new_log_id
    }

    pub fn add_to_write_set(&mut self, log_id: RawLogId) -> Result<()> {
        let not_exists = self.write_set.insert(log_id);
        if !not_exists {
            // Obey single-writer rule
            return_errno_with_msg!(PermissionDenied, "the raw log has more than one writer");
        }
        Ok(())
    }

    pub fn remove_from_write_set(&mut self, log_id: RawLogId) {
        let _is_removed = self.write_set.remove(&log_id);
        // `_is_removed` may equal to `false` if the log has already been deleted
    }
}

impl RawLogStoreState {
    pub fn new() -> Self {
        Self {
            log_table: HashMap::new(),
        }
    }

    pub fn create_log(&mut self, new_log_id: u64) {
        let new_log_entry = RawLogEntry {
            head: RawLogHead::new(),
        };
        let already_exists = self.log_table.insert(new_log_id, new_log_entry).is_some();
        debug_assert!(!already_exists);
    }

    pub(super) fn find_log(&self, log_id: u64) -> Option<RawLogEntry> {
        self.log_table.get(&log_id).cloned()
    }

    pub(super) fn append_log(&mut self, log_id: u64, tail: &RawLogTail) {
        let log_entry = self.log_table.get_mut(&log_id).unwrap();
        log_entry.head.append(tail);
    }

    pub fn delete_log(&mut self, log_id: u64) {
        let _ = self.log_table.remove(&log_id);
        // Leave chunk deallocation to lazy delete
    }
}

impl RawLogHead {
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            num_blocks: 0,
        }
    }

    pub fn append(&mut self, tail: &RawLogTail) {
        // Update head
        self.chunks.extend(tail.chunks.iter());
        self.num_blocks += tail.num_blocks;
        // No need to update tail
    }
}

////////////////////////////////////////////////////////////////////////////////
// Persistent Edit
////////////////////////////////////////////////////////////////////////////////

/// A persistent edit to the state of `RawLogStore`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawLogStoreEdit {
    edit_table: HashMap<RawLogId, RawLogEdit>,
}

/// The basic unit of a persistent edit to the state of `RawLogStore`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) enum RawLogEdit {
    Create(RawLogCreate),
    Append(RawLogAppend),
    Delete,
}

/// An edit that implies a log being created.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RawLogCreate {
    tail: RawLogTail,
}

/// An edit that implies an existing log being appended.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RawLogAppend {
    tail: RawLogTail,
}

/// A log tail contains chunk metadata of a log's TX-ongoing data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RawLogTail {
    // The last chunk of the head. If it is partially filled
    // (head_last_chunk_free_blocks > 0), then the tail should write to the
    // free blocks in the last chunk of the head.
    head_last_chunk_id: ChunkId,
    head_last_chunk_free_blocks: u16,
    // The chunks allocated and owned by the tail
    chunks: Vec<ChunkId>,
    // The total number of blocks in the tail, including the blocks written to
    // the last chunk of head and those written to the chunks owned by the tail.
    num_blocks: u32,
}

impl RawLogStoreEdit {
    /// Creates a new empty edit table.
    pub fn new() -> Self {
        Self {
            edit_table: HashMap::new(),
        }
    }

    /// Records a log creation in the edit.
    pub fn create_log(&mut self, new_log_id: RawLogId) {
        let create_edit = RawLogEdit::Create(RawLogCreate::new());
        let edit_exists = self.edit_table.insert(new_log_id, create_edit);
        debug_assert!(edit_exists.is_none());
    }

    /// Records a log being opened in the edit.
    pub(super) fn open_log(&mut self, log_id: RawLogId, log_entry: &RawLogEntry) {
        match self.edit_table.get(&log_id) {
            None => {
                // Insert an append edit
                let tail = RawLogTail::from_head(&log_entry.head);
                let append_edit = RawLogEdit::Append(RawLogAppend { tail });
                let edit_exists = self.edit_table.insert(log_id, append_edit);
                debug_assert!(edit_exists.is_none());
            }
            Some(edit) => {
                // If `edit == create`, unreachable: there can't be a persistent log entry
                // when the log is just created in an ongoing TX
                if let RawLogEdit::Create(_) = edit {
                    unreachable!();
                }
                // If `edit == append`, do nothing
                // If `edit == delete`, panic
                if let RawLogEdit::Delete = edit {
                    panic!("try to open a deleted log!");
                }
            }
        }
    }

    /// Records a log deletion in the edit, returns the tail chunks of the deleted log.
    pub fn delete_log(&mut self, log_id: RawLogId) -> Option<Vec<ChunkId>> {
        match self.edit_table.insert(log_id, RawLogEdit::Delete) {
            None => None,
            Some(RawLogEdit::Create(create)) => {
                // No need to panic in create
                Some(create.tail.chunks.clone())
            }
            Some(RawLogEdit::Append(append)) => {
                // No need to panic in append (WAL case)
                Some(append.tail.chunks.clone())
            }
            Some(RawLogEdit::Delete) => panic!("try to delete a deleted log!"),
        }
    }

    pub fn is_log_created(&self, log_id: RawLogId) -> bool {
        match self.edit_table.get(&log_id) {
            Some(RawLogEdit::Create(_)) | Some(RawLogEdit::Append(_)) => true,
            Some(RawLogEdit::Delete) | None => false,
        }
    }

    pub fn iter_created_logs(&self) -> impl Iterator<Item = RawLogId> + '_ {
        self.edit_table
            .iter()
            .filter(|(_, edit)| matches!(edit, RawLogEdit::Create(_)))
            .map(|(id, _)| *id)
    }

    pub fn iter_deleted_logs(&self) -> impl Iterator<Item = RawLogId> + '_ {
        self.edit_table
            .iter()
            .filter(|(_, edit)| matches!(edit, RawLogEdit::Delete))
            .map(|(id, _)| *id)
    }

    pub fn is_empty(&self) -> bool {
        self.edit_table.is_empty()
    }
}

impl Edit<RawLogStoreState> for RawLogStoreEdit {
    fn apply_to(&self, state: &mut RawLogStoreState) {
        for (&log_id, log_edit) in self.edit_table.iter() {
            match log_edit {
                RawLogEdit::Create(create) => {
                    let RawLogCreate { tail } = create;
                    state.create_log(log_id);
                    state.append_log(log_id, tail);
                }
                RawLogEdit::Append(append) => {
                    let RawLogAppend { tail } = append;
                    state.append_log(log_id, tail);
                }
                RawLogEdit::Delete => {
                    state.delete_log(log_id);
                }
            }
        }
    }
}

impl RawLogCreate {
    pub fn new() -> Self {
        Self {
            tail: RawLogTail::new(),
        }
    }
}

impl RawLogTail {
    pub fn new() -> Self {
        Self {
            head_last_chunk_id: 0,
            head_last_chunk_free_blocks: 0,
            chunks: Vec::new(),
            num_blocks: 0,
        }
    }

    pub fn from_head(head: &RawLogHead) -> Self {
        Self {
            head_last_chunk_id: *head.chunks.last().unwrap_or(&0),
            head_last_chunk_free_blocks: (head.chunks.len() * CHUNK_NBLOCKS
                - head.num_blocks as usize) as _,
            chunks: Vec::new(),
            num_blocks: 0,
        }
    }
}

impl TxData for RawLogStoreEdit {}

#[cfg(test)]
mod tests {
    use std::thread::{self, JoinHandle};

    use super::*;
    use crate::layers::{
        bio::{Buf, MemDisk},
        log::chunk::{CHUNK_NBLOCKS, CHUNK_SIZE},
    };

    fn create_raw_log_store() -> Result<Arc<RawLogStore<MemDisk>>> {
        let nchunks = 8;
        let nblocks = nchunks * CHUNK_NBLOCKS;
        let tx_provider = TxProvider::new();
        let chunk_alloc = ChunkAlloc::new(nchunks, tx_provider.clone());
        let mem_disk = MemDisk::create(nblocks)?;
        Ok(RawLogStore::new(mem_disk, tx_provider, chunk_alloc))
    }

    fn find_persistent_log_entry(
        log_store: &Arc<RawLogStore<MemDisk>>,
        log_id: RawLogId,
    ) -> Option<RawLogEntry> {
        let state = log_store.state.lock();
        state.persistent.find_log(log_id)
    }

    #[test]
    fn raw_log_store_fns() -> Result<()> {
        let raw_log_store = create_raw_log_store()?;

        // TX 1: create a new log and append contents (committed)
        let mut tx = raw_log_store.new_tx();
        let res: Result<RawLogId> = tx.context(|| {
            let new_log = raw_log_store.create_log()?;
            let mut buf = Buf::alloc(4)?;
            buf.as_mut_slice().fill(2u8);
            new_log.append(buf.as_ref())?;
            assert_eq!(new_log.nblocks(), 4);
            Ok(new_log.id())
        });
        let log_id = res?;
        tx.commit()?;

        let entry = find_persistent_log_entry(&raw_log_store, log_id).unwrap();
        assert_eq!(entry.head.num_blocks, 4);

        // TX 2: open the log, append contents then read (committed)
        let mut tx = raw_log_store.new_tx();
        let res: Result<_> = tx.context(|| {
            let log = raw_log_store.open_log(log_id, true)?;

            let mut buf = Buf::alloc(CHUNK_NBLOCKS)?;
            buf.as_mut_slice().fill(5u8);
            log.append(buf.as_ref())?;

            Ok(())
        });
        res?;

        let res: Result<_> = tx.context(|| {
            let log = raw_log_store.open_log(log_id, true)?;

            let mut buf = Buf::alloc(4)?;
            log.read(1 as BlockId, buf.as_mut())?;
            assert_eq!(&buf.as_slice()[..3 * BLOCK_SIZE], &[2u8; 3 * BLOCK_SIZE]);
            assert_eq!(&buf.as_slice()[3 * BLOCK_SIZE..], &[5u8; BLOCK_SIZE]);

            Ok(())
        });
        res?;
        tx.commit()?;

        let entry = find_persistent_log_entry(&raw_log_store, log_id).unwrap();
        assert_eq!(entry.head.num_blocks, 1028);

        // TX 3: delete the log (committed)
        let mut tx = raw_log_store.new_tx();
        let res: Result<_> = tx.context(|| raw_log_store.delete_log(log_id));
        res?;
        tx.commit()?;

        let entry_opt = find_persistent_log_entry(&raw_log_store, log_id);
        assert!(entry_opt.is_none());

        // TX 4: create a new log (aborted)
        let mut tx = raw_log_store.new_tx();
        let res: Result<_> = tx.context(|| {
            let new_log = raw_log_store.create_log()?;
            Ok(new_log.id())
        });
        let new_log_id = res?;
        tx.abort();

        let entry_opt = find_persistent_log_entry(&raw_log_store, new_log_id);
        assert!(entry_opt.is_none());

        Ok(())
    }

    #[test]
    fn raw_log_deletion() -> Result<()> {
        let raw_log_store = create_raw_log_store()?;

        // Create a new log and append contents
        let mut tx = raw_log_store.new_tx();
        let content = 5_u8;
        let res: Result<_> = tx.context(|| {
            let new_log = raw_log_store.create_log()?;
            let mut buf = Buf::alloc(1)?;
            buf.as_mut_slice().fill(content);
            new_log.append(buf.as_ref())?;
            Ok(new_log.id())
        });
        let log_id = res?;
        tx.commit()?;

        // Concurrently open, read then delete the log
        let handlers = (0..16)
            .map(|_| {
                let raw_log_store = raw_log_store.clone();
                thread::spawn(move || -> Result<()> {
                    let mut tx = raw_log_store.new_tx();
                    println!(
                        "TX[{:?}] executed on thread[{:?}]",
                        tx.id(),
                        crate::os::CurrentThread::id()
                    );
                    let _ = tx.context(|| {
                        let log = raw_log_store.open_log(log_id, false)?;
                        let mut buf = Buf::alloc(1)?;
                        log.read(0 as BlockId, buf.as_mut())?;
                        assert_eq!(buf.as_slice(), &[content; BLOCK_SIZE]);
                        raw_log_store.delete_log(log_id)
                    });
                    tx.commit()
                })
            })
            .collect::<Vec<JoinHandle<Result<()>>>>();
        for handler in handlers {
            handler.join().unwrap()?;
        }

        // The log has already been deleted
        let mut tx = raw_log_store.new_tx();
        let _ = tx.context(|| {
            let res = raw_log_store.open_log(log_id, false).map(|_| ());
            res.expect_err("result must be NotFound");
        });
        tx.commit()
    }
}
