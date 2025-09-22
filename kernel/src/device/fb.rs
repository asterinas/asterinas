// SPDX-License-Identifier: MPL-2.0

use aster_framebuffer::{FrameBuffer, FRAMEBUFFER};

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
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        // TODO: Add support for reading from the framebuffer using `read`
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "reading from the framebuffer is not supported"
        );
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        // TODO: Add support for writing to the framebuffer using `write`
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to the framebuffer is not supported"
        );
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
