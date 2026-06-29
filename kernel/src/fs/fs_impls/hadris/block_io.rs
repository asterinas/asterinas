// SPDX-License-Identifier: MPL-2.0

use aster_block::{BlockDevice, SECTOR_SIZE, bio::BioStatus};
use hadris_io::{
    Error as HadrisIoError, ErrorKind as HadrisIoErrorKind, Result as HadrisIoResult, SeekFrom,
    sync::{Read as HadrisRead, Seek as HadrisSeek, Write as HadrisWrite},
};
use ostd::mm::VmIo;

use crate::prelude::*;

#[derive(Debug)]
pub(super) struct BlockDeviceIo {
    device: Arc<dyn BlockDevice>,
    position: usize,
}

impl BlockDeviceIo {
    pub(super) fn new(device: Arc<dyn BlockDevice>) -> Self {
        Self {
            device,
            position: 0,
        }
    }

    fn device_size(&self) -> usize {
        self.device.metadata().nr_sectors * SECTOR_SIZE
    }

    fn io_error(message: &'static str) -> HadrisIoError {
        HadrisIoError::new(HadrisIoErrorKind::Other, message)
    }
}

impl HadrisRead for BlockDeviceIo {
    fn read(&mut self, buf: &mut [u8]) -> HadrisIoResult<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let device_size = self.device_size();
        if self.position >= device_size {
            return Ok(0);
        }

        let read_len = buf.len().min(device_size - self.position);
        let mut writer = VmWriter::from(&mut buf[..read_len]).to_fallible();
        self.device
            .read(self.position, &mut writer)
            .map_err(|_| Self::io_error("block read failed"))?;
        self.position += read_len;
        Ok(read_len)
    }
}

impl HadrisWrite for BlockDeviceIo {
    fn write(&mut self, buf: &[u8]) -> HadrisIoResult<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let device_size = self.device_size();
        if self.position >= device_size {
            return Err(HadrisIoError::from_kind(HadrisIoErrorKind::WriteZero));
        }

        let write_len = buf.len().min(device_size - self.position);
        let mut reader = VmReader::from(&buf[..write_len]).to_fallible();
        self.device
            .write(self.position, &mut reader)
            .map_err(|_| Self::io_error("block write failed"))?;
        self.position += write_len;
        Ok(write_len)
    }

    fn flush(&mut self) -> HadrisIoResult<()> {
        match self.device.sync() {
            Ok(BioStatus::Complete) => Ok(()),
            _ => Err(Self::io_error("block flush failed")),
        }
    }
}

impl HadrisSeek for BlockDeviceIo {
    fn seek(&mut self, pos: SeekFrom) -> HadrisIoResult<u64> {
        let base = match pos {
            SeekFrom::Start(offset) => {
                self.position = usize::try_from(offset)
                    .map_err(|_| Self::io_error("seek offset is out of range"))?;
                return Ok(offset);
            }
            SeekFrom::End(offset) => self.device_size() as i128 + offset as i128,
            SeekFrom::Current(offset) => self.position as i128 + offset as i128,
        };

        if base < 0 || base > usize::MAX as i128 {
            return Err(Self::io_error("seek offset is out of range"));
        }

        self.position = base as usize;
        Ok(self.position as u64)
    }
}
