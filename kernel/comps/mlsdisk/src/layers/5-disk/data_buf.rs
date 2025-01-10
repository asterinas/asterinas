// SPDX-License-Identifier: MPL-2.0

//! Data buffering.
use core::ops::RangeInclusive;

use super::mlsdisk::RecordKey;
use crate::{
    layers::bio::{BufMut, BufRef},
    os::{BTreeMap, Condvar, CvarMutex, Mutex},
    prelude::*,
};

/// A buffer to cache data blocks before they are written to disk.
#[derive(Debug)]
pub(super) struct DataBuf {
    buf: Mutex<BTreeMap<RecordKey, Arc<DataBlock>>>,
    cap: usize,
    cvar: Condvar,
    is_full: CvarMutex<bool>,
}

/// User data block.
pub(super) struct DataBlock([u8; BLOCK_SIZE]);

impl DataBuf {
    /// Create a new empty data buffer with a given capacity.
    pub fn new(cap: usize) -> Self {
        Self {
            buf: Mutex::new(BTreeMap::new()),
            cap,
            cvar: Condvar::new(),
            is_full: CvarMutex::new(false),
        }
    }

    /// Get the buffered data block with the key and copy
    /// the content into `buf`.
    pub fn get(&self, key: RecordKey, buf: &mut BufMut) -> Option<()> {
        debug_assert_eq!(buf.nblocks(), 1);
        if let Some(block) = self.buf.lock().get(&key) {
            buf.as_mut_slice().copy_from_slice(block.as_slice());
            Some(())
        } else {
            None
        }
    }

    /// Get the buffered data blocks which keys are within the given range.
    pub fn get_range(&self, range: RangeInclusive<RecordKey>) -> Vec<(RecordKey, Arc<DataBlock>)> {
        self.buf
            .lock()
            .iter()
            .filter_map(|(k, v)| {
                if range.contains(k) {
                    Some((*k, v.clone()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Put the data block in `buf` into the buffer. Return
    /// whether the buffer is full after insertion.
    pub fn put(&self, key: RecordKey, buf: BufRef) -> bool {
        debug_assert_eq!(buf.nblocks(), 1);

        let mut is_full = self.is_full.lock().unwrap();
        while *is_full {
            is_full = self.cvar.wait(is_full).unwrap();
        }
        debug_assert!(!*is_full);

        let mut data_buf = self.buf.lock();
        let _ = data_buf.insert(key, DataBlock::from_buf(buf));

        if data_buf.len() >= self.cap {
            *is_full = true;
        }
        *is_full
    }

    /// Return the number of data blocks of the buffer.
    pub fn nblocks(&self) -> usize {
        self.buf.lock().len()
    }

    /// Return whether the buffer is full.
    pub fn at_capacity(&self) -> bool {
        self.nblocks() >= self.cap
    }

    /// Return whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.nblocks() == 0
    }

    /// Empty the buffer.
    pub fn clear(&self) {
        let mut is_full = self.is_full.lock().unwrap();
        self.buf.lock().clear();
        if *is_full {
            *is_full = false;
            self.cvar.notify_all();
        }
    }

    /// Return all the buffered data blocks.
    pub fn all_blocks(&self) -> Vec<(RecordKey, Arc<DataBlock>)> {
        self.buf
            .lock()
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect()
    }
}

impl DataBlock {
    /// Create a new data block from the given `buf`.
    pub fn from_buf(buf: BufRef) -> Arc<Self> {
        debug_assert_eq!(buf.nblocks(), 1);
        Arc::new(DataBlock(buf.as_slice().try_into().unwrap()))
    }

    /// Return the immutable slice of the data block.
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Debug for DataBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataBlock")
            .field("first 16 bytes", &&self.0[..16])
            .finish()
    }
}
