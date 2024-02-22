// SPDX-License-Identifier: MPL-2.0

use aster_frame::vm::{DmaCoherent, DmaStream, DmaStreamSlice, HasDaddr};

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

impl DmaBuf for DmaStreamSlice<'_> {
    fn len(&self) -> usize {
        self.nbytes()
    }
}

impl DmaBuf for DmaCoherent {
    fn len(&self) -> usize {
        self.nbytes()
    }
}
