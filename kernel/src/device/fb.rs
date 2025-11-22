// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_framebuffer::{ColorMapEntry, FrameBuffer, PixelFormat, FRAMEBUFFER, MAX_CMAP_SIZE};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::{
    mm::{io_util::HasVmReaderWriter, HasPaddr, HasSize},
    Pod,
};

use super::char::{self, CharDevice, DevtmpfsName};
use crate::{
    current_userspace,
    events::IoEvents,
    fs::{
        file_handle::Mappable,
        inode_handle::FileIo,
        utils::{InodeIo, IoctlCmd, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

#[derive(Debug)]
struct Fb;

#[derive(Debug)]
struct FbHandle {
    framebuffer: Arc<FrameBuffer>,
}

impl CharDevice for Fb {
    fn devtmpfs_name(&self) -> DevtmpfsName<'_> {
        DevtmpfsName::new("fb0", None)
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux: major 29, minor 0
        DeviceId::new(MajorId::new(29), MinorId::new(0))
    }

    fn open(&self) -> Result<Arc<dyn FileIo>> {
        let Some(framebuffer) = FRAMEBUFFER.get() else {
            return Err(Error::with_message(
                Errno::ENODEV,
                "the framebuffer device is not present",
            ));
        };
        let framebuffer = framebuffer.clone();
        Ok(Arc::new(FbHandle { framebuffer }))
    }
}

impl Pollable for FbHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl InodeIo for FbHandle {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
    }
}

impl FileIo for FbHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn mappable(&self) -> Result<Mappable> {
        let iomem = self.framebuffer.io_mem();
        Ok(Mappable::IoMem(iomem.clone()))
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            _ => {
                log::debug!(
                    "the ioctl command {:?} is not supported by framebuffer devices",
                    cmd
                );
                return_errno_with_message!(
                    Errno::ENOTTY,
                    "the ioctl command is not supported by framebuffer devices"
                )
            }
        }
    }
}

pub(super) fn init_in_first_kthread() {
    if FRAMEBUFFER.get().is_none() {
        return;
    }

    char::register(Arc::new(Fb)).expect("failed to register framebuffer char device");
}
