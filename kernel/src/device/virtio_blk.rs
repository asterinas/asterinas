// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use aster_block::{id::Bid, BlockDevice, BLOCK_SIZE};
use aster_virtio::device::block::device::BlockDevice as VirtioBlkDevice;
use ostd::mm::{FrameAllocOptions, VmIo};

use super::*;
use crate::{
    events::IoEvents,
    fs::{inode_handle::FileIo, utils::IoctlCmd},
    prelude::*,
    process::signal::Poller,
    thread::kernel_thread::KernelThreadExt,
};

/// Spawns a thread for a target block device for handling requests,
/// given the `device_name`.
///
/// Currently only supports virtio-blk devices.
pub fn start_block_device(device_name: &str) -> Result<Arc<dyn BlockDevice>> {
    if let Some(device) = aster_block::get_device(device_name) {
        let cloned_device = device.clone();
        let task_fn = move || {
            let virtio_blk_device = cloned_device.downcast_ref::<VirtioBlkDevice>().unwrap();
            loop {
                virtio_blk_device.handle_requests();
            }
        };
        crate::Thread::spawn_kernel_thread(crate::ThreadOptions::new(task_fn));

        println!("Spawn the virtio-blk device {} thread", device_name);
        Ok(device)
    } else {
        return_errno_with_message!(Errno::ENOENT, "virtio-blk device does not exist");
    }
}

/// Virtio-Blk device. Currently showed as a device under "/dev".
pub(super) struct VirtioBlk {
    device: Arc<dyn BlockDevice>,
}

impl VirtioBlk {
    pub fn get_device(name: &str) -> Self {
        let device = aster_block::get_device(name).unwrap();
        Self { device }
    }
}

impl Device for VirtioBlk {
    fn type_(&self) -> DeviceType {
        DeviceType::BlockDevice
    }

    fn id(&self) -> DeviceId {
        // Consistent with Linux
        DeviceId::new(253, 0)
    }
}

impl FileIo for VirtioBlk {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.read_at(0, writer)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        self.write_at(0, reader)
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let buf_len = writer.avail();
        check_offset_and_buf_len(offset, buf_len)?;

        let buf_nblocks = buf_len / BLOCK_SIZE;
        let segment = FrameAllocOptions::new(buf_nblocks)
            .uninit(true)
            .alloc_contiguous()?;

        self.device
            .read_blocks(Bid::from_offset(offset as _), &segment)?;
        segment.read(0, writer)?;
        Ok(buf_len)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let buf_len = reader.remain();
        check_offset_and_buf_len(offset, buf_len)?;

        let buf_nblocks = buf_len / BLOCK_SIZE;
        let segment = FrameAllocOptions::new(buf_nblocks)
            .uninit(true)
            .alloc_contiguous()?;
        segment.write(0, reader)?;

        self.device
            .write_blocks(Bid::from_offset(offset as _), &segment)?;
        Ok(buf_len)
    }

    fn poll(&self, mask: IoEvents, _poller: Option<&mut Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::BLKBSZGET => {
                CurrentUserSpace::get().write_val(arg as _, &BLOCK_SIZE)?;
                // write_val_to_user(arg, &BLOCK_SIZE)?;
            }
            IoctlCmd::BLKGETSIZE64 => {
                CurrentUserSpace::get().write_val(arg as _, &(1024 * 1024 * 1024_usize))?;
                // write_val_to_user(arg, &(1024 * 1024 * 1024_usize))?;
            }
            _ => {
                return_errno_with_message!(Errno::EINVAL, "ioctl is not supported");
            }
        }
        Ok(0)
    }
}

fn check_offset_and_buf_len(offset: usize, buf_len: usize) -> Result<()> {
    if offset % BLOCK_SIZE != 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid offset");
    }
    if buf_len == 0 || buf_len % BLOCK_SIZE != 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid buffer length");
    }
    Ok(())
}
