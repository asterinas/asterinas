// SPDX-License-Identifier: MPL-2.0

use super::{BlockLog, BlockSet, BufMut, BufRef};
use crate::{os::Mutex, prelude::*};

/// `BlockRing<S>` emulates a blocks log (`BlockLog`) with infinite
/// storage capacity by using a block set (`S: BlockSet`) of finite storage
/// capacity.
///
/// `BlockRing<S>` uses the entire storage space provided by the underlying
/// block set (`S`) for user data, maintaining no extra metadata.
/// Having no metadata, `BlockRing<S>` has to put three responsibilities to
/// its user:
///
/// 1. Tracking the valid block range for read.
///    `BlockRing<S>` accepts reads at any position regardless of whether the
///    position refers to a valid block. It blindly redirects the read request to
///    the underlying block set after moduloing the target position by the
///    size of the block set.
///
/// 2. Setting the cursor for appending new blocks.
///    `BlockRing<S>` won't remember the progress of writing blocks after reboot.
///    Thus, after a `BlockRing<S>` is instantiated, the user must specify the
///    append cursor (using the `set_cursor` method) before appending new blocks.
///
/// 3. Avoiding overriding valid data blocks mistakenly.
///    As the underlying storage is used in a ring buffer style, old
///    blocks must be overridden to accommodate new blocks. The user must ensure
///    that the underlying storage is big enough to avoid overriding any useful
///    data.
pub struct BlockRing<S> {
    storage: S,
    // The cursor for appending new blocks
    cursor: Mutex<Option<BlockId>>,
}

impl<S: BlockSet> BlockRing<S> {
    /// Creates a new instance.
    pub fn new(storage: S) -> Self {
        Self {
            storage,
            cursor: Mutex::new(None),
        }
    }

    /// Set the cursor for appending new blocks.
    ///
    /// # Panics
    ///
    /// Calling the `append` method without setting the append cursor first
    /// via this method `set_cursor` causes panic.
    pub fn set_cursor(&self, new_cursor: BlockId) {
        *self.cursor.lock() = Some(new_cursor);
    }

    // Return a reference to the underlying storage.
    pub fn storage(&self) -> &S {
        &self.storage
    }
}

impl<S: BlockSet> BlockLog for BlockRing<S> {
    fn read(&self, pos: BlockId, buf: BufMut) -> Result<()> {
        let pos = pos % self.storage.nblocks();
        self.storage.read(pos, buf)
    }

    fn append(&self, buf: BufRef) -> Result<BlockId> {
        let cursor = self
            .cursor
            .lock()
            .expect("cursor must be set before appending new blocks");
        let pos = cursor % self.storage.nblocks();
        let new_cursor = cursor + buf.nblocks();
        self.storage.write(pos, buf)?;
        self.set_cursor(new_cursor);
        Ok(cursor)
    }

    fn flush(&self) -> Result<()> {
        self.storage.flush()
    }

    fn nblocks(&self) -> usize {
        self.cursor.lock().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::BlockRing;
    use crate::layers::bio::{BlockLog, Buf, MemDisk};

    #[test]
    fn block_ring() {
        let num_blocks = 16;
        let disk = MemDisk::create(num_blocks).unwrap();
        let block_ring = BlockRing::new(disk);
        block_ring.set_cursor(num_blocks);
        assert_eq!(block_ring.nblocks(), num_blocks);

        let mut append_buf = Buf::alloc(1).unwrap();
        append_buf.as_mut_slice().fill(1);
        let pos = block_ring.append(append_buf.as_ref()).unwrap();
        assert_eq!(pos, num_blocks);
        assert_eq!(block_ring.nblocks(), num_blocks + 1);

        let mut read_buf = Buf::alloc(1).unwrap();
        block_ring
            .read(pos % num_blocks, read_buf.as_mut())
            .unwrap();
        assert_eq!(read_buf.as_slice(), append_buf.as_slice());
    }
}
