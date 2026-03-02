// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use ostd::{
    Result,
    mm::{
        Daddr, FallibleVmWrite, HasDaddr, HasSize, Infallible, VmReader,
        dma::{FromDevice, ToDevice},
    },
};
use ostd_pod::Pod;

use crate::dma_pool::{DmaPool, DmaSegment};

pub struct TxBuffer {
    segment: DmaSegment<ToDevice>,
    nbytes: usize,
}

impl TxBuffer {
    pub fn new<H: Pod>(
        header: &H,
        packet: &mut VmReader,
        pool: &Arc<DmaPool<ToDevice>>,
    ) -> Result<Self> {
        // This wraps `new_impl` to prevent unnecessary monomorphization caused by `H`.
        Self::new_impl(header.as_bytes(), packet, pool)
    }

    fn new_impl(
        header: &[u8],
        packet: &mut VmReader,
        pool: &Arc<DmaPool<ToDevice>>,
    ) -> Result<Self> {
        let nbytes = header.len().checked_add(packet.remain()).unwrap();
        assert!(nbytes <= pool.segment_size());

        let segment = pool.alloc_segment()?;

        let tx_buffer = {
            let mut writer = segment.writer().unwrap();
            writer.write(&mut VmReader::from(header));
            writer.write_fallible(packet).map_err(|(err, _)| err)?;
            Self { segment, nbytes }
        };
        tx_buffer.sync_to_device();

        Ok(tx_buffer)
    }

    fn sync_to_device(&self) {
        self.segment.sync_to_device(0..self.nbytes).unwrap();
    }
}

impl HasSize for TxBuffer {
    fn size(&self) -> usize {
        self.nbytes
    }
}

impl HasDaddr for TxBuffer {
    fn daddr(&self) -> Daddr {
        self.segment.daddr()
    }
}

pub struct RxBuffer {
    segment: DmaSegment<FromDevice>,
    header_len: usize,
    packet_len: usize,
}

impl RxBuffer {
    pub fn new(header_len: usize, pool: &Arc<DmaPool<FromDevice>>) -> Result<Self> {
        assert!(header_len <= pool.segment_size());

        let segment = pool.alloc_segment()?;
        Ok(Self {
            segment,
            header_len,
            packet_len: 0,
        })
    }

    pub const fn packet_len(&self) -> usize {
        self.packet_len
    }

    pub fn set_packet_len(&mut self, packet_len: usize) {
        assert!(self.header_len.checked_add(packet_len).unwrap() <= self.segment.size());
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
