// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::marker::PhantomData;

use ostd::{
    Result,
    mm::{
        Daddr, HasDaddr, HasSize, Infallible, VmReader, VmWriter,
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
    pub fn new<H: Pod>(header: &H, payload: &[u8], pool: &Arc<DmaPool<ToDevice>>) -> Result<Self> {
        let mut builder = Self::new_builder::<H>(pool)?;

        builder
            .copy_payload(|mut writer| {
                assert!(writer.avail() >= payload.len());
                Ok(writer.write(&mut VmReader::from(payload)))
            })
            .unwrap();

        Ok(builder.build(header))
    }

    pub fn new_builder<H: Pod>(pool: &Arc<DmaPool<ToDevice>>) -> Result<TxBufferBuilder<H>> {
        assert!(size_of::<H>() <= pool.segment_size());

        let segment = pool.alloc_segment()?;

        let builder = TxBufferBuilder {
            segment,
            nbytes: size_of::<H>(),
            _phantom: PhantomData,
        };
        Ok(builder)
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

pub struct TxBufferBuilder<H> {
    segment: DmaSegment<ToDevice>,
    nbytes: usize,
    _phantom: PhantomData<H>,
}

impl<H: Pod> TxBufferBuilder<H> {
    pub fn copy_payload<F>(&mut self, copy_fn: F) -> Result<usize>
    where
        F: FnOnce(VmWriter<Infallible>) -> Result<usize>,
    {
        let mut writer = self.segment.writer().unwrap();
        writer.skip(self.nbytes);

        let bytes_written = copy_fn(writer)?;
        self.nbytes += bytes_written;
        debug_assert!(self.nbytes <= self.segment.size());

        Ok(bytes_written)
    }

    pub const fn payload_len(&self) -> usize {
        self.nbytes - size_of::<H>()
    }

    pub fn build(self, header: &H) -> TxBuffer {
        self.segment
            .writer()
            .unwrap()
            .write(&mut VmReader::from(header.as_bytes()));

        let tx_buffer = TxBuffer {
            segment: self.segment,
            nbytes: self.nbytes,
        };
        tx_buffer.sync_to_device();
        tx_buffer
    }
}

pub struct RxBuffer {
    segment: DmaSegment<FromDevice>,
    header_len: usize,
    payload_len: usize,
}

impl RxBuffer {
    pub fn new(header_len: usize, pool: &Arc<DmaPool<FromDevice>>) -> Result<Self> {
        assert!(header_len <= pool.segment_size());

        let segment = pool.alloc_segment()?;
        Ok(Self {
            segment,
            header_len,
            payload_len: 0,
        })
    }

    pub const fn payload_len(&self) -> usize {
        self.payload_len
    }

    pub fn set_payload_len(&mut self, payload_len: usize) {
        assert!(self.header_len.checked_add(payload_len).unwrap() <= self.segment.size());
        self.payload_len = payload_len;
    }

    pub fn payload(&self) -> VmReader<'_, Infallible> {
        self.segment
            .sync_from_device(self.header_len..self.header_len + self.payload_len)
            .unwrap();

        let mut reader = self.segment.reader().unwrap();
        reader.skip(self.header_len).limit(self.payload_len);
        reader
    }

    pub fn buf(&self) -> VmReader<'_, Infallible> {
        self.segment
            .sync_from_device(0..self.header_len + self.payload_len)
            .unwrap();

        let mut reader = self.segment.reader().unwrap();
        reader.limit(self.header_len + self.payload_len);
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
