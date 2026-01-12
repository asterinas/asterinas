// SPDX-License-Identifier: MPL-2.0

use aster_block::BlockDevice;
use aster_virtio::device::block::device::BlockDevice as VirtIoBlockDevice;
use device_id::DeviceId;
use ostd::mm::VmIo;

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceType, add_node},
        inode_handle::FileIo,
        path::PathResolver,
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

pub(super) fn init_in_first_process(path_resolver: &PathResolver) -> Result<()> {
    for device in aster_block::collect_all() {
        let device = Arc::new(BlockFile::new(device));
        if let Some(devtmpfs_path) = device.devtmpfs_path() {
            let dev_id = device.id().as_encoded_u64();
            add_node(DeviceType::Block, dev_id, &devtmpfs_path, path_resolver)?;
        }
    }

    Ok(())
}

/// Represents a block device inode in the filesystem.
//
// TODO: This type wraps an `Arc<dyn BlockDevice>` in another `Arc` just to implement the `Device`
// trait. It leads to redundant vtable dispatch, reference counting, and heap allocation. We should
// devise a better strategy to eliminate the unnecessary intermediate `Arc`.
#[derive(Debug)]
struct BlockFile(Arc<dyn BlockDevice>);

impl BlockFile {
    fn new(device: Arc<dyn BlockDevice>) -> Self {
        Self(device)
    }
}

impl Device for BlockFile {
    fn type_(&self) -> DeviceType {
        DeviceType::Block
    }

    fn id(&self) -> DeviceId {
        self.0.id()
    }

    fn devtmpfs_path(&self) -> Option<String> {
        Some(self.0.name().into())
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(OpenBlockFile(self.0.clone())))
    }
}

/// Represents an opened block device file ready for I/O operations.
//
// TODO: This type wraps an `Arc<dyn BlockDevice>` in another `Box` just to implement the `FileIo`
// trait. It leads to redundant vtable dispatch and heap allocation. We should devise a better
// strategy to eliminate the unnecessary intermediate `Box`.
struct OpenBlockFile(Arc<dyn BlockDevice>);

impl InodeIo for OpenBlockFile {
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

impl Pollable for OpenBlockFile {
    fn poll(&self, mask: IoEvents, _: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for OpenBlockFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }
}

pub(super) fn lookup(id: DeviceId) -> Option<Arc<dyn Device>> {
    let block_device = aster_block::lookup(id)?;

    let mut registry = DEVICE_REGISTRY.lock();
    let block_device_file = registry
        .entry(id.to_raw())
        .or_insert_with(move || Arc::new(BlockFile::new(block_device)))
        .clone();
    Some(block_device_file)
}

// TODO: Merge the two mapping tables, one is here and the other is in the block component.
// Maintaining two mapping tables is undesirable due to duplication and (potential) inconsistency.
static DEVICE_REGISTRY: Mutex<BTreeMap<u32, Arc<dyn Device>>> = Mutex::new(BTreeMap::new());
