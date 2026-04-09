// SPDX-License-Identifier: MPL-2.0

//! Direct Memory Access (DMA).
//!
//! This module provides [`DmaCoherent`] and [`DmaStream`] abstractions for
//! managing DMA memory regions with different remapping, caching and
//! synchronization requirements.
//!
//! # Usage in IRQs
//!
//! Creating DMA objects (via `alloc`, `alloc_uninit`, or `map` constructors)
//! requires IRQs to be enabled, to avoid deadlocks during cross-CPU TLB
//! flushes. Note that it means DMA objects cannot be created from (hard)
//! interrupt context.
//!
//! Other operations on DMA objects may still be performed in any context,
//! even with IRQs disabled. For example, it is valid to drop a [`DmaStream`]
//! from an IRQ handler after the device has finished processing it.

#[cfg(ktest)]
mod test;

mod dma_coherent;
mod dma_stream;
mod util;

pub use dma_coherent::DmaCoherent;
pub use dma_stream::{DmaDirection, DmaStream, FromAndToDevice, FromDevice, ToDevice};
