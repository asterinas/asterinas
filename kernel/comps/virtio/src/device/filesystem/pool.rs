// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::BTreeMap, sync::Arc};
use core::ops::Range;

use aster_network::dma_pool::{DmaPool, DmaSegment};
use aster_util::mem_obj_slice::Slice;
use ostd::{
    Result,
    mm::{
        HasDaddr, HasSize, Infallible, PAGE_SIZE, VmReader, VmWriter,
        dma::{DmaStream, FromDevice, ToDevice},
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
    },
    sync::SpinLock,
};

use crate::{device::VirtioDeviceError, dma_buf::DmaBuf};

const SIZE_CLASSES: &[usize] = &[64, 128, 256, 512, 1024, 2048, 4096];
const POOL_INIT_SIZE: usize = 8;
const POOL_HIGH_WATERMARK: usize = 64;

#[derive(Debug)]
pub struct FsDmaPools {
    to_device_pools: SpinLock<BTreeMap<usize, Arc<DmaPool<ToDevice>>>>,
    from_device_pools: SpinLock<BTreeMap<usize, Arc<DmaPool<FromDevice>>>>,
}

impl FsDmaPools {
    pub fn new() -> Arc<Self> {
        let mut to_device_pools = BTreeMap::new();
        let mut from_device_pools = BTreeMap::new();
        for &class in SIZE_CLASSES {
            to_device_pools.insert(
                class,
                DmaPool::<ToDevice>::new(class, POOL_INIT_SIZE, POOL_HIGH_WATERMARK, false),
            );
            from_device_pools.insert(
                class,
                DmaPool::<FromDevice>::new(class, POOL_INIT_SIZE, POOL_HIGH_WATERMARK, false),
            );
        }

        Arc::new(Self {
            to_device_pools: SpinLock::new(to_device_pools),
            from_device_pools: SpinLock::new(from_device_pools),
        })
    }

    pub fn alloc_to_device(
        self: &Arc<Self>,
        required_len: usize,
    ) -> core::result::Result<FsDmaBuf, VirtioDeviceError> {
        let Some(&class_size) = SIZE_CLASSES.iter().find(|&&size| size >= required_len) else {
            let stream = DmaStream::alloc(required_len.div_ceil(PAGE_SIZE), false)
                .map_err(|_| VirtioDeviceError::QueueUnknownError)?;
            return Ok(FsDmaBuf {
                storage: Arc::new(FsDmaStorage::Stream(Arc::new(stream))),
                required_len,
            });
        };

        let segment = {
            let pools = self.to_device_pools.disable_irq().lock();
            let pool = pools.get(&class_size).unwrap();
            pool.alloc_segment()
                .map_err(|_| VirtioDeviceError::QueueUnknownError)
        }?;

        Ok(FsDmaBuf {
            storage: Arc::new(FsDmaStorage::ToSegment(segment)),
            required_len,
        })
    }

    pub fn alloc_from_device(
        self: &Arc<Self>,
        required_len: usize,
    ) -> core::result::Result<FsDmaBuf, VirtioDeviceError> {
        let Some(&class_size) = SIZE_CLASSES.iter().find(|&&size| size >= required_len) else {
            let stream = DmaStream::alloc(required_len.div_ceil(PAGE_SIZE), false)
                .map_err(|_| VirtioDeviceError::QueueUnknownError)?;
            return Ok(FsDmaBuf {
                storage: Arc::new(FsDmaStorage::Stream(Arc::new(stream))),
                required_len,
            });
        };

        let segment = {
            let pools = self.from_device_pools.disable_irq().lock();
            let pool = pools.get(&class_size).unwrap();
            pool.alloc_segment()
                .map_err(|_| VirtioDeviceError::QueueUnknownError)
        }?;

        Ok(FsDmaBuf {
            storage: Arc::new(FsDmaStorage::FromSegment(segment)),
            required_len,
        })
    }
}

#[derive(Debug)]
enum FsDmaStorage {
    ToSegment(DmaSegment<ToDevice>),
    FromSegment(DmaSegment<FromDevice>),
    Stream(Arc<DmaStream>),
}

#[derive(Debug, Clone)]
pub struct FsDmaBuf {
    storage: Arc<FsDmaStorage>,
    required_len: usize,
}

impl FsDmaBuf {
    pub fn sync_from_device(&self, byte_range: Range<usize>) -> Result<()> {
        match self.storage.as_ref() {
            // To-device buffers are not expected to be synced from device.
            FsDmaStorage::ToSegment(_) => Ok(()),
            FsDmaStorage::FromSegment(segment) => segment.sync_from_device(byte_range),
            FsDmaStorage::Stream(stream) => stream.sync_from_device(byte_range),
        }
    }

    pub fn sync_to_device(&self, byte_range: Range<usize>) -> Result<()> {
        match self.storage.as_ref() {
            FsDmaStorage::ToSegment(segment) => segment.sync_to_device(byte_range),
            // From-device buffers are not expected to be synced to device.
            FsDmaStorage::FromSegment(_) => Ok(()),
            FsDmaStorage::Stream(stream) => stream.sync_to_device(byte_range),
        }
    }
}

impl HasSize for FsDmaBuf {
    fn size(&self) -> usize {
        self.required_len
    }
}

impl HasDaddr for FsDmaBuf {
    fn daddr(&self) -> ostd::mm::Daddr {
        match self.storage.as_ref() {
            FsDmaStorage::ToSegment(segment) => segment.daddr(),
            FsDmaStorage::FromSegment(segment) => segment.daddr(),
            FsDmaStorage::Stream(stream) => stream.daddr(),
        }
    }
}

impl HasVmReaderWriter for FsDmaBuf {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> ostd::prelude::Result<VmReader<'_, Infallible>> {
        let mut reader = match self.storage.as_ref() {
            FsDmaStorage::ToSegment(segment) => segment.reader()?,
            FsDmaStorage::FromSegment(segment) => segment.reader()?,
            FsDmaStorage::Stream(stream) => stream.reader()?,
        };
        reader.limit(self.required_len);
        Ok(reader)
    }

    fn writer(&self) -> ostd::prelude::Result<VmWriter<'_, Infallible>> {
        let mut writer = match self.storage.as_ref() {
            FsDmaStorage::ToSegment(segment) => segment.writer()?,
            FsDmaStorage::FromSegment(segment) => segment.writer()?,
            FsDmaStorage::Stream(stream) => stream.writer()?,
        };
        writer.limit(self.required_len);
        Ok(writer)
    }
}

impl DmaBuf for FsDmaBuf {
    fn len(&self) -> usize {
        self.size()
    }
}

impl DmaBuf for Slice<FsDmaBuf> {
    fn len(&self) -> usize {
        self.size()
    }
}
