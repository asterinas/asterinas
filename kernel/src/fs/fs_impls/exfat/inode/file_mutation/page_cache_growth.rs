// SPDX-License-Identifier: MPL-2.0

//! Owns boundary-page preparation for regular-file page-cache growth.
//!
//! This child module isolates the narrow page-cache seam used by regular-file growth.
//! It prepares only the boundary pages that must exist before zero-fill or user-copy work
//! can expose newly grown bytes through the shared page cache.
//!
//! Its entry point accepts the current data length and a candidate growth range
//! and derives the start and end page boundaries that need preparation.
//! The helper assumes higher layers already own the validated cluster-map generation
//! and the correct publication order for callback-visible page-cache context.
//!
//! This module is intentionally limited.
//! It does not own generic whole-range preparation,
//! cluster allocation,
//! or entry-set persistence,
//! and it relies on callers to preserve the dirty-page and growth-order premises around it.

use core::ops::Range;

use ostd::mm::io::util::HasVmReaderWriter;

use super::super::ExfatInode;
use crate::{prelude::*, vm::page_cache::PageCache};

impl ExfatInode {
    pub(super) fn prepare_regular_file_page_cache_boundary_pages(
        page_cache: &PageCache,
        current_data_length: usize,
        range: Range<usize>,
    ) -> Result<()> {
        if range.is_empty() {
            return Ok(());
        }

        let vmo = page_cache.as_vmo().clone();
        let prepare_page_fn = |page_idx: usize| -> Result<()> {
            let frame = vmo.commit_on(page_idx)?;
            frame.writer().fill_zeros(PAGE_SIZE);
            Ok(())
        };

        let start_page_idx = range.start / PAGE_SIZE;
        let start_page_offset = start_page_idx
            .checked_mul(PAGE_SIZE)
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        if !range.start.is_multiple_of(PAGE_SIZE) && start_page_offset >= current_data_length {
            prepare_page_fn(start_page_idx)?;
        }

        if !range.end.is_multiple_of(PAGE_SIZE) {
            let end_page_idx = range.end / PAGE_SIZE;
            let end_page_offset = end_page_idx
                .checked_mul(PAGE_SIZE)
                .ok_or_else(|| Error::new(Errno::EINVAL))?;
            if end_page_offset >= current_data_length
                && (end_page_idx != start_page_idx || range.start.is_multiple_of(PAGE_SIZE))
            {
                prepare_page_fn(end_page_idx)?;
            }
        }
        Ok(())
    }
}
