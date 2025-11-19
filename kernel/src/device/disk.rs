// SPDX-License-Identifier: MPL-2.0

use aster_block::BlockDevice;
use aster_virtio::device::block::device::BlockDevice as VirtIoBlockDevice;
use device_id::DeviceId;
use ostd::mm::VmIo;

use crate::{
    events::IoEvents,
    fs::{
        device::{add_node, Device, DeviceType},
        fs_resolver::FsResolver,
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    thread::kernel_thread::ThreadOptions,
};

pub(super) fn init_in_first_kthread() {
    for device in aster_block::collect_all() {
        if device.is_partition() {
            continue;
        }

        let task_fn = move || {
            info!("spawn the virt-io-block thread");
            let virtio_block_device = device.downcast_ref::<VirtIoBlockDevice>().unwrap();
            loop {
                virtio_block_device.handle_requests();
            }
        };
        ThreadOptions::new(task_fn).spawn();
    }
}

pub(super) fn init_in_first_process(fs_resolver: &FsResolver) -> Result<()> {
    for device in aster_block::collect_all() {
        let name = device.name().to_string();
        let device = Arc::new(BlockFile::new(device));
        add_node(device, &name, fs_resolver)?;
    }

    Ok(())
}

#[derive(Debug)]
struct BlockFile(Arc<dyn BlockDevice>);

impl BlockFile {
    fn new(device: Arc<dyn BlockDevice>) -> Self {
        Self(device)
    }
}

impl InodeIo for BlockFile {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let total = writer.avail();
        self.0.read(offset, writer)?;
        let avail = writer.avail();
        Ok(total - avail)
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let total = reader.remain();
        self.0.write(offset, reader)?;
        let remain = reader.remain();
        Ok(total - remain)
    }
}

impl Pollable for BlockFile {
    fn poll(&self, mask: IoEvents, _: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for BlockFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }
}

impl Device for BlockFile {
    fn type_(&self) -> DeviceType {
        DeviceType::Block
    }

    fn id(&self) -> DeviceId {
        self.0.id()
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(BlockFile(self.0.clone())))
    }
}
