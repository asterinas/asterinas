// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::linked_list::LinkedList, sync::Arc};

use ostd::{
    mm::{
        Daddr, DmaDirection, DmaStream, FrameAllocOptions, HasDaddr, Infallible, VmReader,
        VmWriter, PAGE_SIZE,
    },
    sync::{LocalIrqDisabled, SpinLock},
    Pod,
};
use spin::Once;

use crate::dma_pool::{DmaPool, DmaSegment};

pub struct TxBuffer {
    dma_stream: DmaStream,
    nbytes: usize,
    pool: &'static SpinLock<LinkedList<DmaStream>, LocalIrqDisabled>,
}

impl TxBuffer {
    pub fn new<H: Pod>(
        header: &H,
        packet: &[u8],
        pool: &'static SpinLock<LinkedList<DmaStream>, LocalIrqDisabled>,
    ) -> Self {
        let header = header.as_bytes();
        let nbytes = header.len() + packet.len();

        assert!(nbytes <= TX_BUFFER_LEN);

        let dma_stream = if let Some(stream) = pool.lock().pop_front() {
            stream
        } else {
            let segment = FrameAllocOptions::new(TX_BUFFER_LEN / PAGE_SIZE)
                .alloc_contiguous()
                .unwrap();
            DmaStream::map(segment, DmaDirection::ToDevice, false).unwrap()
        };

        let tx_buffer = {
            let mut writer = dma_stream.writer().unwrap();
            writer.write(&mut VmReader::from(header));
            writer.write(&mut VmReader::from(packet));
            Self {
                dma_stream,
                nbytes,
                pool,
            }
        };

        tx_buffer.sync();
        tx_buffer
    }

    pub fn writer(&self) -> VmWriter<'_, Infallible> {
        self.dma_stream.writer().unwrap().limit(self.nbytes)
    }

    fn sync(&self) {
        self.dma_stream.sync(0..self.nbytes).unwrap();
    }

    pub fn nbytes(&self) -> usize {
        self.nbytes
    }
}

impl HasDaddr for TxBuffer {
    fn daddr(&self) -> Daddr {
        self.dma_stream.daddr()
    }
}

impl Drop for TxBuffer {
    fn drop(&mut self) {
        self.pool.lock().push_back(self.dma_stream.clone());
    }
}

pub struct RxBuffer {
    segment: DmaSegment,
    header_len: usize,
    packet_len: usize,
}

impl RxBuffer {
    pub fn new(header_len: usize, pool: &Arc<DmaPool>) -> Self {
        assert!(header_len <= pool.segment_size());
        let segment = pool.alloc_segment().unwrap();
        Self {
            segment,
            header_len,
            packet_len: 0,
        }
    }

    pub const fn packet_len(&self) -> usize {
        self.packet_len
    }

    pub fn set_packet_len(&mut self, packet_len: usize) {
        assert!(self.header_len + packet_len <= RX_BUFFER_LEN);
        self.packet_len = packet_len;
    }

    pub fn packet(&self) -> VmReader<'_, Infallible> {
        self.segment
            .sync(self.header_len..self.header_len + self.packet_len)
            .unwrap();
        self.segment
            .reader()
            .unwrap()
            .skip(self.header_len)
            .limit(self.packet_len)
    }

    pub fn buf(&self) -> VmReader<'_, Infallible> {
        self.segment
            .sync(0..self.header_len + self.packet_len)
            .unwrap();
        self.segment
            .reader()
            .unwrap()
            .limit(self.header_len + self.packet_len)
    }

    pub const fn buf_len(&self) -> usize {
        self.segment.size()
    }
}

impl HasDaddr for RxBuffer {
    fn daddr(&self) -> Daddr {
        self.segment.daddr()
    }
}

pub const RX_BUFFER_LEN: usize = 4096;
pub const TX_BUFFER_LEN: usize = 4096;
pub static RX_BUFFER_POOL: Once<Arc<DmaPool>> = Once::new();

pub fn init() {
    const POOL_INIT_SIZE: usize = 64;
    const POOL_HIGH_WATERMARK: usize = 128;
    RX_BUFFER_POOL.call_once(|| {
        DmaPool::new(
            RX_BUFFER_LEN,
            POOL_INIT_SIZE,
            POOL_HIGH_WATERMARK,
            DmaDirection::FromDevice,
            false,
        )
    });
}
