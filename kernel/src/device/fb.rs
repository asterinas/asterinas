// SPDX-License-Identifier: MPL-2.0

pub(crate) use aster_framebuffer::get_framebuffer_info;
use aster_framebuffer::FrameBuffer;

use super::*;
use crate::{
    events::IoEvents,
    fs::{file_handle::Mappable, inode_handle::FileIo, utils::IoctlCmd},
    prelude::*,
    process::signal::{PollHandle, Pollable},
};
pub struct Fb;

impl Fb {
    /// Get the framebuffer instance or return an error if not initialized.
    fn get_framebuffer(&self) -> Result<Arc<FrameBuffer>> {
        get_framebuffer_info().ok_or_else(|| {
            Error::with_message(Errno::ENODEV, "Framebuffer has not been initialized")
        })
    }
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
        Ok(Some(Arc::new(Fb)))
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
        // Reading from framebuffer is not supported
        return_errno_with_message!(Errno::ENOSYS, "Fb: read is not supported");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        // Writing to the framebuffer device is not supported.
        return_errno_with_message!(Errno::EINVAL, "Writing to framebuffer is not supported");
    }

    fn mappable(&self) -> Result<Mappable> {
        let framebuffer = self.get_framebuffer()?;
        let iomem = framebuffer.io_mem();
        Ok(Mappable::IoMem(iomem.clone()))
    }

    fn ioctl(&self, cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        log::debug!("Fb ioctl: Unsupported command -> {:?}", cmd);
        return_errno!(Errno::EINVAL);
    }
}
