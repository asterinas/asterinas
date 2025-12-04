// SPDX-License-Identifier: MPL-2.0

//! Direct Memory Access (DMA) memory management.
//!
//! This module provides [`DmaCoherent`] and [`DmaStream`] abstractions for
//! managing DMA memory regions with different remapping, caching and
//! synchronization requirements.

#[cfg(ktest)]
mod test;

mod dma_coherent;
mod dma_stream;
mod util;

pub use dma_coherent::DmaCoherent;
pub use dma_stream::{Bidirectional, DmaDirection, DmaStream, FromDevice, ToDevice};
