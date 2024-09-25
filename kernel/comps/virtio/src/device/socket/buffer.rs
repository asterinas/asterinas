// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::linked_list::LinkedList, sync::Arc};

use aster_network::dma_pool::DmaPool;
use ostd::{
    mm::{DmaDirection, DmaStream},
    sync::{LocalIrqDisabled, SpinLock},
};
use spin::Once;

const RX_BUFFER_LEN: usize = 4096;
const TX_BUFFER_LEN: usize = 4096;
pub static RX_BUFFER_POOL: Once<Arc<DmaPool>> = Once::new();
pub static TX_BUFFER_POOL: Once<SpinLock<LinkedList<DmaStream>, LocalIrqDisabled>> = Once::new();

pub fn init() {
    const POOL_INIT_SIZE: usize = 32;
    const POOL_HIGH_WATERMARK: usize = 64;
    RX_BUFFER_POOL.call_once(|| {
        DmaPool::new(
            RX_BUFFER_LEN,
            POOL_INIT_SIZE,
            POOL_HIGH_WATERMARK,
            DmaDirection::FromDevice,
            false,
        )
    });
    TX_BUFFER_POOL.call_once(|| SpinLock::new(LinkedList::new()));
}
