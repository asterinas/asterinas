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
