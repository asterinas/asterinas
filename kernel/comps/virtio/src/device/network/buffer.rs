// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_network::dma_pool::DmaPool;
use ostd::mm::dma::{FromDevice, ToDevice};
use spin::Once;

const RX_BUFFER_LEN: usize = 4096;
const TX_BUFFER_LEN: usize = 4096;

pub(super) static RX_BUFFER_POOL: Once<Arc<DmaPool<FromDevice>>> = Once::new();
pub(super) static TX_BUFFER_POOL: Once<Arc<DmaPool<ToDevice>>> = Once::new();

pub(super) fn init() {
    const POOL_INIT_SIZE: usize = 32;
    const POOL_HIGH_WATERMARK: usize = 64;

    RX_BUFFER_POOL
        .call_once(|| DmaPool::new(RX_BUFFER_LEN, POOL_INIT_SIZE, POOL_HIGH_WATERMARK, false));
    TX_BUFFER_POOL
        .call_once(|| DmaPool::new(TX_BUFFER_LEN, POOL_INIT_SIZE, POOL_HIGH_WATERMARK, false));
}
