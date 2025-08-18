// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_network::{DmaSegment, RxBuffer, TxBuffer};
use aster_util::mem_obj_slice::Slice;
use ostd::mm::{DmaCoherent, DmaStream, HasDaddr, HasSize};

/// A DMA-capable buffer.
///
/// Any type implements this trait should also implements `HasDaddr` trait,
/// and provides the exact length of DMA area.
pub trait DmaBuf: HasDaddr {
    /// The length of Dma area, in bytes
    fn len(&self) -> usize;
}

macro_rules! impl_dma_buf_for {
    ($($t:ty),*) => {
        $(
            impl DmaBuf for $t {
                fn len(&self) -> usize {
                    self.size()
                }
            }

            impl DmaBuf for Slice<$t> {
                fn len(&self) -> usize {
                    self.size()
                }
            }
        )*
    };
}

impl_dma_buf_for!(
    DmaStream,
    &DmaStream,
    Arc<DmaStream>,
    &Arc<DmaStream>,
    DmaCoherent,
    &DmaCoherent,
    Arc<DmaCoherent>,
    &Arc<DmaCoherent>
);

impl DmaBuf for DmaSegment {
    fn len(&self) -> usize {
        self.size()
    }
}

impl DmaBuf for TxBuffer {
    fn len(&self) -> usize {
        self.size()
    }
}

impl DmaBuf for RxBuffer {
    fn len(&self) -> usize {
        self.size()
    }
}
