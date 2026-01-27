// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::linked_list::LinkedList, sync::Arc};

use aster_softirq::BottomHalfDisabled;
use ostd::{
    mm::{
        Daddr, FrameAllocOptions, HasDaddr, HasSize, Infallible, PAGE_SIZE, VmReader, VmWriter,
        dma::{DmaStream, FromDevice, ToDevice},
        io_util::HasVmReaderWriter,
    },
    sync::SpinLock,
};
use ostd_pod::Pod;
use spin::Once;

use crate::dma_pool::{DmaPool, DmaSegment};

pub struct TxBuffer {
    dma_stream: Arc<DmaStream<ToDevice>>,
    nbytes: usize,
    pool: &'static SpinLock<LinkedList<Arc<DmaStream<ToDevice>>>, BottomHalfDisabled>,
}

impl TxBuffer {
    pub fn new<H: Pod>(
        header: &H,
        packet: &[u8],
        pool: &'static SpinLock<LinkedList<Arc<DmaStream<ToDevice>>>, BottomHalfDisabled>,
    ) -> Self {
        let header = header.as_bytes();
        let nbytes = header.len() + packet.len();

        assert!(nbytes <= TX_BUFFER_LEN);

        let dma_stream = if let Some(stream) = pool.lock().pop_front() {
            stream
        } else {
            let segment = FrameAllocOptions::new()
                .alloc_segment(TX_BUFFER_LEN / PAGE_SIZE)
                .unwrap();
            Arc::new(DmaStream::<ToDevice>::map(segment.into(), false).unwrap())
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

        tx_buffer.sync_to_device();
        tx_buffer
    }

    pub fn writer(&self) -> VmWriter<'_, Infallible> {
        let mut writer = self.dma_stream.writer().unwrap();
        writer.limit(self.nbytes);
        writer
    }

    fn sync_to_device(&self) {
        self.dma_stream.sync_to_device(0..self.nbytes).unwrap();
    }
}

impl HasSize for TxBuffer {
    fn size(&self) -> usize {
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
    segment: DmaSegment<FromDevice>,
    header_len: usize,
    packet_len: usize,
}

impl RxBuffer {
    pub fn new(header_len: usize, pool: &Arc<DmaPool<FromDevice>>) -> Self {
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
            .sync_from_device(self.header_len..self.header_len + self.packet_len)
            .unwrap();
        let mut reader = self.segment.reader().unwrap();
        reader.skip(self.header_len).limit(self.packet_len);
        reader
    }

    pub fn buf(&self) -> VmReader<'_, Infallible> {
        self.segment
            .sync_from_device(0..self.header_len + self.packet_len)
            .unwrap();
        let mut reader = self.segment.reader().unwrap();
        reader.limit(self.header_len + self.packet_len);
        reader
    }
}

impl HasSize for RxBuffer {
    fn size(&self) -> usize {
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
pub static RX_BUFFER_POOL: Once<Arc<DmaPool<FromDevice>>> = Once::new();

pub fn init() {
    const POOL_INIT_SIZE: usize = 64;
    const POOL_HIGH_WATERMARK: usize = 128;
    RX_BUFFER_POOL.call_once(|| {
        DmaPool::<FromDevice>::new(RX_BUFFER_LEN, POOL_INIT_SIZE, POOL_HIGH_WATERMARK, false)
    });
}
