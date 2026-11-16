// SPDX-License-Identifier: MPL-2.0

//! Instance-local immutable raw VMO pages for metadata and inline-tail reads.
//!
//! Pages remain in the VMO for reuse and failed initialization can be retried.

use core::fmt;

use aster_block::BlockDevice;
use ostd::mm::VmWriter;

use super::io::{self, RawPageBackend};
use crate::{
    prelude::{Arc, Errno, Result},
    vm::page_cache::PageCache,
};

pub(super) struct MetadataCache {
    // Fields drop in declaration order: retire cached pages before the backend.
    cache: PageCache,
    // The VMO keeps only a Weak backend; this owner covers all synchronous reads.
    backend: Arc<RawPageBackend>,
}
impl fmt::Debug for MetadataCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetadataCache")
            .field("nr_sectors", &self.nr_sectors())
            .finish_non_exhaustive()
    }
}
impl MetadataCache {
    pub(super) fn new(device: Arc<dyn BlockDevice>) -> ostd::Result<Self> {
        let backend = Arc::new(RawPageBackend::new(device)?);
        let cache = backend.create_cache()?;
        Ok(Self { cache, backend })
    }

    pub(super) fn backend(&self) -> &Arc<RawPageBackend> {
        &self.backend
    }

    pub(super) fn nr_sectors(&self) -> usize {
        self.backend.nr_sectors()
    }

    /// Reads a complete byte window, preserving native backend errors.
    pub(super) fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        // VMO reads may truncate at its aligned end. Reject requests outside
        // the actual device, including empty windows beyond that boundary.
        io::check_read_range(self.nr_sectors(), offset, writer.avail())?;
        self.cache.as_vmo().read(offset, writer, None)
    }

    pub(super) fn read_bytes(&self, offset: usize, output: &mut [u8]) -> ostd::Result<()> {
        // Classify range errors before translating backend enqueue failures.
        io::check_read_range(self.nr_sectors(), offset, output.len())?;
        self.cache
            .as_vmo()
            .read(offset, &mut VmWriter::from(output).to_fallible(), None)
            .map_err(|error| match error.error() {
                // The window is in bounds; EINVAL now means an oversized BIO.
                Errno::EINVAL | Errno::EBUSY => ostd::Error::NotEnoughResources,
                Errno::EOPNOTSUPP | Errno::ENOSPC => ostd::Error::IoError,
                _ => ostd::Error::from(error),
            })
    }
}
