// SPDX-License-Identifier: MPL-2.0

//! File reads, canonical cache acquisition, and checked flat-page mappings.
//!
//! The inode's backend initializes flat or compressed output cache pages.
//! Validated layouts provide exact file lengths and accepted physical ranges.
//! Page mappings use those checked ranges without validating the layout again.
//! Inline reads need only the tail's bytes, including on a short device page.
//! The supported filesystem block size equals `PAGE_SIZE` (4096 bytes).
//!
//! Flat mapping reference:
//! <https://github.com/torvalds/linux/blob/5dd1818b15d98d4a20806cd00b1b40320b06004f/fs/erofs/data.c#L148>.

use ostd::{Error, mm};

use super::layout::{InodeDataLayout, PlainBlocks};
use crate::{
    prelude::{Arc, Errno, Error as KernelError, Result as KernelResult},
    vm::page_cache::PageCache,
};

/// A flat-page mapping checked by `for_page` for one stable device snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct FlatPageMapping {
    kind: FlatPageMappingKind,
}

/// A read-only description of the location carried by a checked mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum FlatPageMappingKind {
    /// A plain block or inline prefix, with data starting at target-page byte 0.
    DataBlock {
        /// Byte offset of the read block from the start of the supplied block device.
        device_start_bytes: usize,
    },
    /// A metadata block whose selected bytes start at target-page byte 0.
    InlineTail {
        /// Byte offset of the read block from the start of the supplied block device.
        device_start_bytes: usize,
        /// Byte offset of the inline file data within that read block.
        source_offset_bytes: usize,
        valid_bytes: usize,
    },
    Eof,
}

impl FlatPageMapping {
    /// Describes the checked location without allowing a mapping to be forged.
    pub(in super::super) fn kind(self) -> FlatPageMappingKind {
        self.kind
    }

    /// Maps one logical page within a checked data layout.
    ///
    /// Rejects non-data layouts before address arithmetic. Unrepresentable
    /// logical page offsets return `Overflow`.
    /// Representable pages at or beyond EOF need no physical range check.
    pub(in super::super) fn for_page(
        layout: InodeDataLayout,
        page_idx: usize,
    ) -> ostd::Result<Self> {
        let size_bytes = match layout {
            InodeDataLayout::Plain(plain) => plain.size_bytes(),
            InodeDataLayout::Inline(inline) => inline.size_bytes(),
            InodeDataLayout::Compressed(_)
            | InodeDataLayout::Device(_)
            | InodeDataLayout::NoData => return Err(Error::InvalidArgs),
        };
        // Byte offset of this logical page within the file, not within the device.
        let page_start_bytes = page_idx.checked_mul(mm::PAGE_SIZE).ok_or(Error::Overflow)?;
        let size_bytes = usize::try_from(size_bytes).map_err(|_| Error::Overflow)?;
        if page_start_bytes >= size_bytes {
            return Ok(Self {
                kind: FlatPageMappingKind::Eof,
            });
        }
        match layout {
            // Nonempty checked layouts have an allocation (plain) or tail (inline).
            InodeDataLayout::Plain(plain) => {
                Ok(Self::data_block(plain.blocks().unwrap(), page_start_bytes))
            }
            InodeDataLayout::Inline(inline) => {
                // Page indices and block counts are comparable because both sizes are 4096.
                if let Some(prefix) = inline.prefix()
                    && page_idx < prefix.block_count()
                {
                    return Ok(Self::data_block(prefix, page_start_bytes));
                }

                let tail = inline.tail().unwrap();
                let source_offset_bytes = tail.start_bytes() % mm::PAGE_SIZE;
                // A remainder cannot exceed its nonnegative dividend.
                let device_start_bytes = tail.start_bytes() - source_offset_bytes;
                Ok(Self {
                    kind: FlatPageMappingKind::InlineTail {
                        device_start_bytes,
                        source_offset_bytes,
                        valid_bytes: tail.length_bytes(),
                    },
                })
            }
            // These layouts were rejected before page arithmetic.
            InodeDataLayout::Compressed(_)
            | InodeDataLayout::Device(_)
            | InodeDataLayout::NoData => unreachable!(),
        }
    }

    /// Selects a block inside the layout's already checked allocation.
    fn data_block(blocks: PlainBlocks, page_start_bytes: usize) -> Self {
        // for_page bounded this offset by the logical size; layout construction
        // checked the entire allocation, including its final block and endpoint.
        let device_start_bytes = blocks.start_bytes() + page_start_bytes;
        Self {
            kind: FlatPageMappingKind::DataBlock { device_start_bytes },
        }
    }
}

/// Checks both representation and alignment before the upstream align_up.
pub(in super::super) fn checked_cache_capacity(size_bytes: u64) -> ostd::Result<usize> {
    let size_bytes = usize::try_from(size_bytes).map_err(|_| Error::Overflow)?;
    size_bytes
        .checked_next_multiple_of(mm::PAGE_SIZE)
        .ok_or(Error::Overflow)
}

impl super::ErofsInode {
    /// Returns the canonical output cache for an ordinary file.
    pub(in super::super) fn file_cache(&self) -> Option<Arc<PageCache>> {
        use crate::fs::file::InodeType;

        (self.description().inode_type() == InodeType::File).then(|| self.data_cache.clone())
    }

    /// Returns the shared flat cache for files, directories, and symbolic links.
    ///
    /// This checks storage eligibility, not directory or link semantics. All
    /// three callers use this same cache; special inodes reject content access.
    pub(in super::super) fn flat_cache(&self) -> KernelResult<Arc<PageCache>> {
        if !matches!(
            self.description().data_layout(),
            InodeDataLayout::Plain(_) | InodeDataLayout::Inline(_)
        ) {
            return Err(KernelError::new(Errno::EINVAL));
        }
        Ok(self.data_cache.clone())
    }
}

impl super::ErofsInode {
    /// Reads ordinary-file bytes through the canonical output page cache.
    ///
    /// Direct I/O is unsupported, including an empty request. Request overflow
    /// is checked before EOF clamping. A successful prefix is returned if a
    /// later error occurs. No atime changes are made.
    pub(in super::super) fn read_at(
        &self,
        offset: usize,
        writer: &mut mm::VmWriter,
        status_flags: crate::fs::file::StatusFlags,
        readahead: Option<&crate::vm::page_cache::Readahead>,
    ) -> KernelResult<usize> {
        use crate::fs::file::{InodeType, StatusFlags};

        match self.description().inode_type() {
            InodeType::File => (),
            InodeType::Dir => return Err(KernelError::new(Errno::EISDIR)),
            _ => return Err(KernelError::new(Errno::EINVAL)),
        }

        if status_flags.contains(StatusFlags::O_DIRECT) {
            return Err(KernelError::new(Errno::EOPNOTSUPP));
        }

        let request_end = offset.checked_add(writer.avail()).ok_or(Error::Overflow)?;
        // Checked Plain, Inline, and Compressed layouts guarantee usize-sized contents.
        let size = self.description().size_bytes() as usize;
        let read_end = request_end.min(size);
        if offset >= read_end {
            return Ok(0);
        }
        let read_len = read_end - offset;

        let (result, copied) = {
            let mut limited = writer.clone_exclusive();
            limited.limit(read_len);
            // The PageCache VmIo wrapper reduces errno through ostd::Error.
            // Its own VMO's native Result preserves backend/completion errors.
            let result = self
                .data_cache
                .as_vmo()
                .read(offset, &mut limited, readahead);
            let copied = read_len - limited.avail();
            (result, copied)
        };

        writer.skip(copied);
        // Only bytes delivered to the caller count as progress. A later page
        // or user-copy failure must preserve that successful prefix.
        match result {
            Err(error) if copied == 0 => Err(error),
            _ => Ok(copied),
        }
    }
}
