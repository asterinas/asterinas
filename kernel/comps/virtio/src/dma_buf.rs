// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_network::{DmaSegment, RxBuffer, TxBuffer};
use aster_util::mem_obj_slice::Slice;
use ostd::mm::{
    HasDaddr, HasSize,
    dma::{DmaCoherent, DmaDirection, DmaStream},
};

/// A DMA-capable buffer.
///
/// Any type implements this trait should also implements `HasDaddr` trait,
/// and provides the exact length of DMA area.
pub trait DmaBuf: HasDaddr {
    /// The length of Dma area, in bytes
    fn len(&self) -> usize;
}

macro_rules! impl_dma_buf_for {
    (<D> $t:ty) => {
        impl<D: DmaDirection> DmaBuf for $t {
            fn len(&self) -> usize {
                self.size()
            }
        }

        impl<D: DmaDirection> DmaBuf for Slice<$t> {
            fn len(&self) -> usize {
                self.size()
            }
        }
    };
    ($t:ty) => {
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
    };
}

impl_dma_buf_for!(<D> DmaStream<D>);
impl_dma_buf_for!(<D> &DmaStream<D>);
impl_dma_buf_for!(<D> Arc<DmaStream<D>>);
impl_dma_buf_for!(<D> &Arc<DmaStream<D>>);
impl_dma_buf_for!(DmaCoherent);
impl_dma_buf_for!(&DmaCoherent);
impl_dma_buf_for!(Arc<DmaCoherent>);
impl_dma_buf_for!(&Arc<DmaCoherent>);

impl<D: DmaDirection> DmaBuf for DmaSegment<D> {
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
