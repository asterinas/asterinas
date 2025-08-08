// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicU32;

use aster_block::BlockDevice;
use aster_virtio::device::block::device::BlockDevice as VirtIoBlockDevice;
use ostd::mm::VmIo;

use super::{
    add_node,
    partition::{parse_partitions, PartitionNode},
    DeviceId,
};
use crate::{
    events::IoEvents,
    fs::inode_handle::FileIo,
    prelude::*,
    process::signal::{PollHandle, Pollable},
    thread::kernel_thread::ThreadOptions,
};

/// The major device number assigned to virtio block devices.
pub(super) const VIRTIO_DEVICE_MAJOR: u32 = 253;

/// The number of minor device numbers allocated for each virtio disk,
/// including the whole disk and its partitions. If a disk has more than
/// 16 partitions, the extended major:minor numbers will be assigned.
pub(super) const VIRTIO_DEVICE_MINORS: u32 = 16;

/// The major device number used for extended partitions when the number
/// of disk partitions exceeds the standard limit.
pub(super) const EXTENDED_MAJOR: u32 = 259;

/// The next available minor device number for extended partitions.
pub(super) static EXTENDED_MINOR: AtomicU32 = AtomicU32::new(0);

/// Represents a disk device node (e.g., "/dev/vda").
#[derive(Debug, Clone)]
pub(super) struct DiskNode {
    id: DeviceId,
    name: String,
    device: Weak<dyn BlockDevice>,
    partitions: Vec<Option<Arc<PartitionNode>>>,
}

impl DiskNode {
    pub(super) fn name(&self) -> &str {
        self.name.as_str()
    }

    pub(super) fn device(&self) -> Option<Arc<dyn BlockDevice>> {
        self.device.upgrade()
    }

    pub(super) fn partition(&self, index: usize) -> Option<Arc<dyn BlockDevice>> {
        if index == 0 || index > self.partitions.len() {
            return None;
        }

        self.partitions[index - 1]
            .as_ref()
            .map(|p| p.clone() as Arc<dyn BlockDevice>)
    }
}

impl super::Device for DiskNode {
    fn id(&self) -> DeviceId {
        self.id
    }

    fn type_(&self) -> super::DeviceType {
        super::DeviceType::BlockDevice
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(Arc::new(self.clone())))
    }
}

impl FileIo for DiskNode {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let Some(device) = self.device() else {
            return_errno_with_message!(Errno::EIO, "device is gone");
        };

        let total = writer.avail();
        device.read(0, writer)?;
        let avail = writer.avail();
        Ok(total - avail)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let Some(device) = self.device() else {
            return_errno_with_message!(Errno::EIO, "device is gone");
        };

        let total = reader.remain();
        device.write(0, reader)?;
        let remain = reader.remain();
        Ok(total - remain)
    }
}

impl Pollable for DiskNode {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

fn add_disk_node(index: usize, name: &String, device: &Arc<dyn BlockDevice>) {
    let cloned_device = device.clone();
    let task_fn = move || {
        info!("spawn the virt-io-block thread");
        let virtio_block_device = cloned_device.downcast_ref::<VirtIoBlockDevice>().unwrap();
        loop {
            virtio_block_device.handle_requests();
        }
    };
    ThreadOptions::new(task_fn).spawn();

    let id = DeviceId::new(VIRTIO_DEVICE_MAJOR, VIRTIO_DEVICE_MINORS * index as u32);

    let partitions = parse_partitions(device, &id, name);

    let disk_node = Arc::new(DiskNode {
        id,
        device: Arc::downgrade(device),
        name: name.clone(),
        partitions,
    });
    let _ = add_node(disk_node.clone(), name.as_str());
    DISK_REGISTRY.lock().push(disk_node);
}

pub(super) static DISK_REGISTRY: SpinLock<Vec<Arc<DiskNode>>> = SpinLock::new(Vec::new());

pub(super) fn init() {
    for (i, (name, device)) in aster_block::all_devices().iter().enumerate() {
        add_disk_node(i, name, device);
    }
}
