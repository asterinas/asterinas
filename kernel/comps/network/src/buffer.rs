// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::LinkedList, sync::Arc};

use align_ext::AlignExt;
use ostd::{
    mm::{
        Daddr, DmaDirection, DmaStream, FrameAllocOptions, HasDaddr, VmReader, VmWriter, PAGE_SIZE,
    },
    sync::SpinLock,
    Pod,
};
use spin::Once;

use crate::dma_pool::{DmaPool, DmaSegment};

pub struct TxBuffer {
    dma_stream: DmaStream,
    nbytes: usize,
    pool: &'static SpinLock<LinkedList<DmaStream>>,
}

impl TxBuffer {
    pub fn new<H: Pod>(
        header: &H,
        packet: &[u8],
        pool: &'static SpinLock<LinkedList<DmaStream>>,
    ) -> Self {
        let header = header.as_bytes();
        let nbytes = header.len() + packet.len();

        let dma_stream = if let Some(stream) = get_tx_stream_from_pool(nbytes, pool) {
            stream
        } else {
            let segment = {
                let nframes = (nbytes.align_up(PAGE_SIZE)) / PAGE_SIZE;
                FrameAllocOptions::new(nframes).alloc_contiguous().unwrap()
            };
            DmaStream::map(segment, DmaDirection::ToDevice, false).unwrap()
        };

        let mut writer = dma_stream.writer().unwrap();
        writer.write(&mut VmReader::from(header));
        writer.write(&mut VmReader::from(packet));

        let tx_buffer = Self {
            dma_stream,
            nbytes,
            pool,
        };
        tx_buffer.sync();
        tx_buffer
    }

    pub fn writer(&self) -> VmWriter<'_> {
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
        self.pool
            .lock_irq_disabled()
            .push_back(self.dma_stream.clone());
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

    pub fn packet(&self) -> VmReader<'_> {
        self.segment
            .sync(self.header_len..self.header_len + self.packet_len)
            .unwrap();
        self.segment
            .reader()
            .unwrap()
            .skip(self.header_len)
            .limit(self.packet_len)
    }

    pub fn buf(&self) -> VmReader<'_> {
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

const RX_BUFFER_LEN: usize = 4096;
pub static RX_BUFFER_POOL: Once<Arc<DmaPool>> = Once::new();
pub static TX_BUFFER_POOL: Once<SpinLock<LinkedList<DmaStream>>> = Once::new();

fn get_tx_stream_from_pool(
    nbytes: usize,
    tx_buffer_pool: &'static SpinLock<LinkedList<DmaStream>>,
) -> Option<DmaStream> {
    let mut pool = tx_buffer_pool.lock_irq_disabled();
    let mut cursor = pool.cursor_front_mut();
    while let Some(current) = cursor.current() {
        if current.nbytes() >= nbytes {
            return cursor.remove_current();
        }
        cursor.move_next();
    }

    None
}

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
