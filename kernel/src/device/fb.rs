// SPDX-License-Identifier: MPL-2.0

use aster_framebuffer::{FrameBuffer, FRAMEBUFFER};
use ostd::mm::{HasSize, VmIo};

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        file_handle::Mappable,
        inode_handle::FileIo,
        utils::IoctlCmd,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

pub struct Fb;

pub struct FbHandle {
    framebuffer: Arc<FrameBuffer>,
    offset: Mutex<usize>,
}

impl Device for Fb {
    fn type_(&self) -> DeviceType {
        DeviceType::Misc
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux
        DeviceId::new(29, 0)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        let framebuffer = FRAMEBUFFER
            .get()
            .cloned()
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "there is no framebuffer device"))?;

        let handle = FbHandle {
            framebuffer,
            offset: Mutex::new(0),
        };

        Ok(Some(Arc::new(handle)))
    }
}

impl Pollable for Fb {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for Fb {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EBADF, "device not opened");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EBADF, "device not opened");
    }

    fn mappable(&self) -> Result<Mappable> {
        return_errno_with_message!(Errno::EBADF, "device not opened");
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        return_errno_with_message!(Errno::EBADF, "device not opened");
    }
}

impl FileIo for FbHandle {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let buffer_size = self.framebuffer.io_mem().size();
        let mut offset = self.offset.lock();

        if *offset >= buffer_size {
            return Ok(0); // EOF
        }

        let read_len = writer.avail().min(buffer_size - *offset);
        if read_len == 0 {
            return Ok(0);
        }

        // Read from the framebuffer at the current offset.
        // Limit the writer to avoid over-reading when the user buffer is
        // larger than the remaining framebuffer size.
        self.framebuffer
            .io_mem()
            .read(*offset, writer.limit(read_len))?;

        *offset += read_len;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let buffer_size = self.framebuffer.io_mem().size();
        let mut offset = self.offset.lock();

        if *offset >= buffer_size {
            return_errno_with_message!(Errno::ENOSPC, "write beyond framebuffer size");
        }

        let write_len = reader.remain().min(buffer_size - *offset);
        if write_len == 0 {
            return Ok(0);
        }

        // Write to the framebuffer at the current offset.
        // Limit the reader to avoid over-writing when the user buffer is
        // larger than the remaining framebuffer size.
        self.framebuffer
            .io_mem()
            .write(*offset, reader.limit(write_len))?;

        *offset += write_len;
        Ok(write_len)
    }

    fn mappable(&self) -> Result<Mappable> {
        let iomem = self.framebuffer.io_mem();
        Ok(Mappable::IoMem(iomem.clone()))
    }

    fn ioctl(&self, cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        log::debug!("Fb ioctl: Unsupported command -> {:?}", cmd);
        return_errno!(Errno::EINVAL);
    }
}

impl Pollable for FbHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
