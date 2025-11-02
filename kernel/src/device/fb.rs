// SPDX-License-Identifier: MPL-2.0

use aster_framebuffer::{ColorMapEntry, FrameBuffer, PixelFormat, FRAMEBUFFER, MAX_CMAP_SIZE};
use ostd::{
    mm::{io_util::HasVmReaderWriter, HasPaddr, HasSize, VmIo},
    Pod,
};

use crate::{
    current_userspace,
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        file_handle::Mappable,
        inode_handle::FileIo,
        utils::{IoctlCmd, StatusFlags},
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

    fn open(&self) -> Option<Result<Arc<dyn FileIo>>> {
        let Some(framebuffer) = FRAMEBUFFER.get() else {
            return Some(Err(Error::with_message(
                Errno::ENODEV,
                "the framebuffer device is not present",
            )));
        };
        let framebuffer = framebuffer.clone();

        let handle: Arc<dyn FileIo> = Arc::new(FbHandle {
            framebuffer,
            offset: Mutex::new(0),
        });

        Some(Ok(handle))
    }
}

impl Pollable for Fb {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for Fb {
    fn read(&self, _writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EBADF, "device not opened");
    }

    fn write(&self, _reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
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
    fn read(&self, writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        if !writer.has_avail() {
            return Ok(0);
        }

        let mut reader = self.framebuffer.io_mem().reader();
        let mut offset = self.offset.lock();

        if *offset >= reader.remain() {
            return Ok(0);
        }
        reader.skip(*offset);

        let mut reader = reader.to_fallible();
        let len = match reader.read_fallible(writer) {
            Ok(len) => len,
            Err((err, 0)) => return Err(err.into()),
            Err((_err, len)) => len,
        };
        *offset += len;

        Ok(len)
    }

    fn write(&self, reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        if !reader.has_remain() {
            return Ok(0);
        }

        let mut writer = self.framebuffer.io_mem().writer();
        let mut offset = self.offset.lock();

        if *offset >= writer.avail() {
            return_errno_with_message!(
                Errno::ENOSPC,
                "the write offset is beyond the framebuffer size"
            );
        }
        writer.skip(*offset);

        let mut writer = writer.to_fallible();
        let len = match writer.write_fallible(reader) {
            Ok(len) => len,
            Err((err, 0)) => return Err(err.into()),
            Err((_err, len)) => len,
        };

        *offset += len;
        Ok(len)
    }

    fn mappable(&self) -> Result<Mappable> {
        let iomem = self.framebuffer.io_mem();
        Ok(Mappable::IoMem(iomem.clone()))
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {}
}

impl Pollable for FbHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
