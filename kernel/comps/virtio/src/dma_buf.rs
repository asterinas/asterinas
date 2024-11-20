// SPDX-License-Identifier: MPL-2.0

use aster_network::{DmaSegment, RxBuffer, TxBuffer};
use ostd::mm::{DmaCoherent, DmaStream, DmaStreamSlice, HasDaddr};

/// A DMA-capable buffer.
///
/// Any type implements this trait should also implements `HasDaddr` trait,
/// and provides the exact length of DMA area.
#[allow(clippy::len_without_is_empty)]
pub trait DmaBuf: HasDaddr {
    /// The length of Dma area, in bytes
    fn len(&self) -> usize;
}

impl DmaBuf for DmaStream {
    fn len(&self) -> usize {
        self.nbytes()
    }
}

impl<Dma: AsRef<DmaStream>> DmaBuf for DmaStreamSlice<Dma> {
    fn len(&self) -> usize {
        self.nbytes()
    }
}

impl DmaBuf for DmaCoherent {
    fn len(&self) -> usize {
        self.nbytes()
    }
}

impl DmaBuf for DmaSegment {
    fn len(&self) -> usize {
        self.size()
    }
}

impl DmaBuf for TxBuffer {
    fn len(&self) -> usize {
        self.nbytes()
    }
}

impl DmaBuf for RxBuffer {
    fn len(&self) -> usize {
        self.buf_len()
    }
}
