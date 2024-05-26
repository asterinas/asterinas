// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_frame::vm::{Daddr, DmaDirection, HasDaddr, VmReader, VmWriter};
use pod::Pod;
use spin::Once;

use crate::dma_pool::{DmaPool, DmaSegment};

pub struct TxBuffer {
    dma_segment: DmaSegment,
    nbytes: usize,
    header_len: usize,
}

impl TxBuffer {
    pub fn new<H: Pod>(header: &H, packet: &mut VmReader) -> Self {
        let header_len = core::mem::size_of::<H>();
        assert!(header_len + packet.remain() <= TX_BUFFER_LEN);

        let header = header.as_bytes();

        let dma_segment = TX_BUFFER_POOL
            .get()
            .unwrap()
            .alloc_segment()
            .expect("fail to allocate dma block");

        let tx_buffer = {
            let mut writer = dma_segment.writer().unwrap();
            writer.write(&mut VmReader::from(header));
            let packet_len = writer.write(packet);
            let nbytes = header.len() + packet_len;
            Self {
                dma_segment,
                nbytes,
                header_len,
            }
        };

        tx_buffer.sync();
        tx_buffer
    }

    pub fn set_packet(&mut self, packet: &mut VmReader) {
        assert!(packet.remain() + self.header_len <= TX_BUFFER_LEN);

        let mut writer = self.dma_segment.writer().unwrap().skip(self.header_len);
        let len = writer.write(packet);
        self.nbytes = self.header_len + len;
        self.dma_segment.sync(self.header_len..self.nbytes).unwrap();
    }

    pub fn clear_packet(&mut self) {
        self.nbytes = self.header_len;
    }

    pub const fn contains_packet(&self) -> bool {
        self.nbytes > self.header_len
    }

    pub fn writer(&self) -> VmWriter<'_> {
        self.dma_segment.writer().unwrap().limit(self.nbytes)
    }

    fn sync(&self) {
        self.dma_segment.sync(0..self.nbytes).unwrap();
    }

    pub const fn nbytes(&self) -> usize {
        self.nbytes
    }
}

impl HasDaddr for TxBuffer {
    fn daddr(&self) -> Daddr {
        self.dma_segment.daddr()
    }
}

pub struct RxBuffer {
    segment: DmaSegment,
    header_len: usize,
    packet_len: usize,
}

impl RxBuffer {
    pub fn new(header_len: usize) -> Self {
        assert!(header_len <= RX_BUFFER_LEN);
        let segment = RX_BUFFER_POOL.get().unwrap().alloc_segment().unwrap();
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
const TX_BUFFER_LEN: usize = 2048;
static RX_BUFFER_POOL: Once<Arc<DmaPool>> = Once::new();
static TX_BUFFER_POOL: Once<Arc<DmaPool>> = Once::new();

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
    TX_BUFFER_POOL.call_once(|| {
        DmaPool::new(
            TX_BUFFER_LEN,
            POOL_INIT_SIZE,
            POOL_HIGH_WATERMARK,
            DmaDirection::ToDevice,
            false,
        )
    });
}
