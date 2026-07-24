// SPDX-License-Identifier: MPL-2.0

//! Ext4 linear directory read operations: `lookup` and `readdir`.
//!
//! A directory inode is page-cache-backed like a regular file; these methods
//! read its data blocks through the page cache and parse linear directory
//! entries. The htree index (Phase 6) only accelerates lookup; `readdir`
//! always walks blocks in physical order.

mod dir_entry;
mod hash;
mod htree;

use self::{
    dir_entry::{DirBlockView, DirEntryFileType, DirEntryHeader},
    htree::DxCtx,
};
use super::{
    super::{fs::Ext4, prelude::*},
    FileFlags, Inode, InodeInner,
};

/// A located, live directory entry returned by [`InodeInner::find_entry_info`].
#[derive(Clone, Copy, Debug)]
struct DirEntryInfo {
    /// Inode number of the located entry. Read by `lookup` to fetch the child
    /// inode.
    ino: Ext4Ino,
}

/// Builds the htree hash context for a lookup on `fs`, or `None` when the volume
/// has no `dir_index` feature (so every directory is scanned linearly). A
/// directory that carries the `INDEX` flag is then probed via this context.
fn dx_ctx(fs: &Ext4) -> Option<DxCtx> {
    let sb = fs.super_block();
    sb.has_dir_index().then(|| DxCtx {
        seed: *sb.hash_seed(),
        unsigned: sb.hash_unsigned(),
    })
}

impl InodeInner {
    /// Iterates entries from byte `offset`, feeding each active entry to
    /// `visitor`. Returns the number of bytes advanced.
    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.desc.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let size = self.file_size();
        let min_rec_len = DirEntryHeader::min_rec_len(1) as usize;
        if size < min_rec_len || offset > size - min_rec_len {
            return Ok(0);
        }

        let start_block = offset / BLOCK_SIZE;
        let mut current_offset = offset;
        let mut advanced = 0usize;
        let total_blocks = size.div_ceil(BLOCK_SIZE);
        let page_cache = self.page_cache()?;

        for block_idx in start_block..total_blocks {
            let block_offset = block_idx * BLOCK_SIZE;
            if block_offset >= size {
                break;
            }
            let block = DirBlockView::from_index(page_cache, block_idx, size);
            let mut iter = block.iter_entries();
            while let Some((entry_offset_in_block, entry)) = iter.next_entry()? {
                let entry_offset = block_offset + entry_offset_in_block;
                let rec_len = entry.header.rec_len as usize;
                let next_offset = entry_offset + rec_len;

                // Skip entries already reported before `offset`.
                if next_offset <= current_offset {
                    continue;
                }
                if entry_offset < current_offset {
                    current_offset = next_offset;
                    continue;
                }

                let ino = entry.header.ino;
                if ino != 0 {
                    let name = core::str::from_utf8(entry.name)
                        .map_err(|_| Error::with_message(Errno::EIO, "invalid dir entry name"))?;
                    let file_type = DirEntryFileType::try_from(entry.header.file_type)
                        .unwrap_or(DirEntryFileType::Unknown);
                    let inode_type = InodeType::from(file_type);
                    if visitor
                        .visit(name, ino as u64, inode_type, next_offset)
                        .is_err()
                    {
                        return Ok(current_offset - offset);
                    }
                }
                current_offset = next_offset;
            }
            advanced = current_offset - offset;
        }
        Ok(advanced)
    }

    /// Scans one directory logical block for `name`, returning the located entry
    /// or `None` if it is not in this block.
    fn scan_block_for_name(
        &self,
        block_idx: usize,
        name_bytes: &[u8],
    ) -> Result<Option<DirEntryInfo>> {
        let file_size = self.file_size();
        let block = DirBlockView::from_index(self.page_cache()?, block_idx, file_size);
        let mut iter = block.iter_entries();
        while let Some((_, entry)) = iter.next_entry()? {
            let ino = entry.header.ino;
            if ino == 0 || entry.name != name_bytes {
                continue;
            }
            return Ok(Some(DirEntryInfo { ino }));
        }
        Ok(None)
    }

    /// Locates a live entry by name and returns its inode number.
    ///
    /// On a `dir_index` volume `dx` carries the hash inputs; when this directory
    /// also has the `INDEX` flag, the htree index is probed to the single leaf
    /// block that may hold the name and only that block is scanned (O(log n)). A
    /// probe miss — an unsupported hash, a corrupt index, or a hash collision
    /// that spilled the name into a neighbouring leaf — falls through to the
    /// linear scan, which is always correct: the index is an accelerator, never
    /// the source of truth (Linux falls back the same way on `ERR_BAD_DX_DIR`).
    fn find_entry_info(&self, name: &str, dx: Option<&DxCtx>) -> Result<DirEntryInfo> {
        if self.desc.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let name_bytes = name.as_bytes();

        if let Some(dx) = dx
            && self.desc.flags().contains(FileFlags::INDEX)
        {
            let page_cache = self.page_cache()?;
            let read_block = |logical: Ext4Bid| -> Result<[u8; BLOCK_SIZE]> {
                page_cache
                    .read_val(logical as usize * BLOCK_SIZE)
                    .map_err(|_| {
                        Error::with_message(Errno::EIO, "failed to read htree index block")
                    })
            };
            if let Ok(Some(leaf)) =
                htree::dx_lookup_leaf(read_block, name_bytes, &dx.seed, dx.unsigned)
                // A corrupt index can point a leaf past EOF; validate the block
                // against the directory size before scanning so a bad pointer
                // falls through to the linear scan below rather than driving an
                // out-of-range read (Linux rejects `block >= i_size` up front in
                // `__ext4_read_dirblock`).
                && leaf < self.file_size().div_ceil(BLOCK_SIZE) as Ext4Bid
                && let Some(info) = self.scan_block_for_name(leaf as usize, name_bytes)?
            {
                return Ok(info);
            }
        }

        let file_size = self.file_size();
        for block_idx in 0..file_size.div_ceil(BLOCK_SIZE) {
            if let Some(info) = self.scan_block_for_name(block_idx, name_bytes)? {
                return Ok(info);
            }
        }
        return_errno!(Errno::ENOENT)
    }
}

impl Inode {
    /// Looks up a child entry by name and reads its inode.
    pub(in crate::fs::fs_impls::ext4) fn lookup(&self, name: &str) -> Result<Arc<Inode>> {
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem dropped"))?;
        let ino = self
            .inner
            .read()
            .find_entry_info(name, dx_ctx(&fs).as_ref())?
            .ino;
        fs.read_inode(ino)
    }

    /// Iterates directory entries from `offset`, feeding them to `visitor`.
    pub(in crate::fs::fs_impls::ext4) fn readdir_at(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<usize> {
        self.inner.read().readdir_at(offset, visitor)
    }
}

#[cfg(ktest)]
mod tests {
    use alloc::{
        string::{String, ToString},
        vec::Vec,
    };

    use ostd::prelude::*;

    use super::super::super::test_utils::{
        Ext4FixtureBuilder, make_dir_block, make_dir_inode, make_file_inode,
    };
    use crate::{
        fs::{file::InodeType, utils::DirentVisitor},
        prelude::{Errno, Error, Result},
    };

    #[ktest]
    fn lookup_and_readdir() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();

        // A directory at ino 12 backed by data block 101 with three entries.
        let dir_block = 101u32;
        let block = make_dir_block(&[(2, ".", 2), (2, "..", 2), (11, "hello.txt", 1)]);
        f.write_data_block(dir_block, &block);
        f.write_raw_inode(12, &make_dir_inode(dir_block));
        // The looked-up file inode must exist to be read back.
        f.write_raw_inode(11, &make_file_inode(100, 0));

        let dir = f.ext4.read_inode(12).unwrap();

        let child = dir.lookup("hello.txt").unwrap();
        assert_eq!(child.ino(), 11);
        assert_eq!(child.inode_type(), InodeType::File);
        assert!(dir.lookup("nonexistent").is_err());

        let mut names: Vec<String> = Vec::new();
        dir.readdir_at(0, &mut names).unwrap();
        assert_eq!(names.len(), 3);
        assert_eq!(names[0], ".");
        assert_eq!(names[1], "..");
        assert_eq!(names[2], "hello.txt");
    }

    /// A visitor that simulates a `getdents` buffer holding `cap` entries: it
    /// records each entry, then fails (as a full user buffer would) once `cap`
    /// entries have been accepted in the current call.
    struct CappedVisitor {
        cap: usize,
        used: usize,
        names: Vec<String>,
    }

    impl DirentVisitor for CappedVisitor {
        fn visit(
            &mut self,
            name: &str,
            _ino: u64,
            _type_: InodeType,
            _offset: usize,
        ) -> Result<()> {
            if self.used >= self.cap {
                return Err(Error::with_message(Errno::EINVAL, "simulated buffer full"));
            }
            self.used += 1;
            self.names.push(name.to_string());
            Ok(())
        }
    }

    /// Drives `readdir_at` exactly the way `InodeHandle::readdir` does — advancing
    /// the directory offset by the returned byte count across multiple calls.
    /// This exercises offset resumption that the single-call test above cannot,
    /// mirroring a real mke2fs root layout (`.`, `..`, `lost+found`, files).
    #[ktest]
    fn readdir_resumes_across_getdents_calls() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let dir_block = 101u32;
        let block = make_dir_block(&[
            (2, ".", 2),
            (2, "..", 2),
            (11, "lost+found", 2),
            (12, "hello.txt", 1),
            (13, "subdir", 2),
            (15, "big.txt", 1),
        ]);
        f.write_data_block(dir_block, &block);
        f.write_raw_inode(12, &make_dir_inode(dir_block));
        let dir = f.ext4.read_inode(12).unwrap();

        let expected = [".", "..", "lost+found", "hello.txt", "subdir", "big.txt"];

        // Including one entry per call, which forces the most resumption steps.
        for cap in [1usize, 2, 3, 6] {
            let mut all: Vec<String> = Vec::new();
            let mut offset = 0usize;
            loop {
                let mut visitor = CappedVisitor {
                    cap,
                    used: 0,
                    names: Vec::new(),
                };
                let read_cnt = dir.readdir_at(offset, &mut visitor).unwrap();
                all.extend(visitor.names);
                if read_cnt == 0 {
                    break;
                }
                offset += read_cnt;
                assert!(
                    all.len() <= expected.len(),
                    "readdir over-reported (cap={cap})"
                );
            }
            assert_eq!(all, expected, "readdir mismatch at cap={cap}");
        }
    }
}
