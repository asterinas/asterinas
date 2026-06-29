// SPDX-License-Identifier: MPL-2.0

use aster_block::{BLOCK_SIZE, BlockDevice, SECTOR_SIZE};
use aster_nvme::NvmeBlockDevice;
use aster_virtio::device::block::device::BlockDevice as VirtIoBlockDevice;
use device_id::DeviceId;
use ostd::mm::VmIo;

use crate::{
    context::current_userspace,
    device::{Device, DeviceType, DevtmpfsInodeMeta, add_node},
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        vfs::{inode::FileOps, path::PathResolver},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    thread::kernel_thread::ThreadOptions,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

/// Legacy Linux disk geometry returned by `HDIO_GETGEO`.
///
/// GRUB uses the `start` field to map Linux partition devices back to GRUB
/// partitions when sysfs does not expose `/sys/dev/block/<major>:<minor>/start`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct HdGeometry {
    heads: u8,
    sectors: u8,
    cylinders: u16,
    _padding: u32,
    start: u64,
}

pub(super) fn init_in_first_kthread() {
    for device in aster_block::collect_all() {
        if device.is_partition() {
            continue;
        }

        // Spawn threads for virtio block devices
        if device.downcast_ref::<VirtIoBlockDevice>().is_some() {
            let device_clone = device.clone();
            let task_fn = move || {
                info!("spawn the virtio-block thread");
                let virtio_block_device = device_clone.downcast_ref::<VirtIoBlockDevice>().unwrap();
                loop {
                    virtio_block_device.handle_requests();
                }
            };
            ThreadOptions::new(task_fn).spawn();
        }
        // Spawn threads for NVMe block devices
        else if device.downcast_ref::<NvmeBlockDevice>().is_some() {
            let device_clone = device.clone();
            let task_fn = move || {
                info!("spawn the nvme-block thread");
                let nvme_block_device = device_clone.downcast_ref::<NvmeBlockDevice>().unwrap();
                loop {
                    nvme_block_device.handle_requests();
                }
            };
            ThreadOptions::new(task_fn).spawn();
        }
    }
}

pub(super) fn init_in_first_process(path_resolver: &PathResolver) -> Result<()> {
    for device in aster_block::collect_all() {
        let device = Arc::new(BlockFile::new(device));
        if let Some(devtmpfs_meta) = device.devtmpfs_meta() {
            let dev_id = device.id().as_encoded_u64();
            add_node(DeviceType::Block, dev_id, &devtmpfs_meta, path_resolver)?;
        }
    }

    Ok(())
}

mod ioctl_defs {
    use super::HdGeometry;
    use crate::util::ioctl::{NoData, OutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/hdreg.h>

    /// Returns legacy disk geometry.
    pub(super) type HdIoGetGeo = ioc!(HDIO_GETGEO, 0x0301, OutData<HdGeometry>);

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/fs.h>

    /// Returns the device size in 512-byte sectors.
    pub(super) type BlkGetSize = ioc!(BLKGETSIZE, 0x1260, OutData<u64>);

    /// Returns the device size in bytes.
    pub(super) type BlkGetSize64 = ioc!(BLKGETSIZE64, 0x12, 114, OutData<u64>);

    /// Returns the direct-I/O alignment size reported for the block device.
    ///
    /// Asterinas block I/O is currently backed by page-sized blocks, so
    /// reporting the 512-byte sector size would make tools issue `O_DIRECT`
    /// I/O that the filesystem layer rejects.
    pub(super) type BlkGetSectorSize = ioc!(BLKSSZGET, 0x12, 104, NoData);

    /// Re-reads the partition table.
    pub(super) type BlkRrPart = ioc!(BLKRRPART, 0x12, 95, NoData);

    /// Partition table modification.
    pub(super) type BlkPg = ioc!(BLKPG, 0x12, 105, NoData);

    /// Flushes buffers.
    pub(super) type BlkFlsBuf = ioc!(BLKFLSBUF, 0x12, 97, NoData);
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

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        Some(DevtmpfsInodeMeta::new(self.0.name()))
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(OpenBlockFile(self.0.clone())))
    }
}

/// Represents an opened block device file ready for I/O operations.
//
// TODO: This type wraps an `Arc<dyn BlockDevice>` in another `Box` just to implement the
// `PerOpenFileOps` trait. It leads to redundant vtable dispatch and heap allocation. We should
// devise a better strategy to eliminate the unnecessary intermediate `Box`.
struct OpenBlockFile(Arc<dyn BlockDevice>);

impl FileOps for OpenBlockFile {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let total = writer.avail();
        if total == 0 {
            return Ok(0);
        }

        let device_size = self.0.metadata().nr_sectors * SECTOR_SIZE;
        if offset >= device_size {
            return Ok(0);
        }

        let read_len = total.min(device_size - offset);
        {
            // `VmIo::read` does not allow short writes,
            // so the writer must be precisely limited here.
            let mut limited_writer = writer.clone_exclusive();
            limited_writer.limit(read_len);
            self.0.read(offset, &mut limited_writer)?;
        }
        writer.skip(read_len);
        Ok(read_len)
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let total = reader.remain();
        if total == 0 {
            return Ok(0);
        }

        let device_size = self.0.metadata().nr_sectors * SECTOR_SIZE;
        if offset >= device_size {
            return_errno_with_message!(
                Errno::ENOSPC,
                "the write offset is beyond the block device"
            );
        }

        let write_len = total.min(device_size - offset);
        {
            // `VmIo::write` does not allow short writes,
            // so the reader must be precisely limited here.
            let mut limited_reader = reader.clone();
            limited_reader.limit(write_len);
            self.0.write(offset, &mut limited_reader)?;
        }
        reader.skip(write_len);
        Ok(write_len)
    }
}

impl Pollable for OpenBlockFile {
    fn poll(&self, mask: IoEvents, _: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl PerOpenFileOps for OpenBlockFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn seek_end(&self) -> Result<Option<usize>> {
        Ok(Some(self.0.metadata().nr_sectors * SECTOR_SIZE))
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            _cmd @ BlkGetSectorSize => {
                // TODO: Report the per-device logical sector size once both block-device and
                // filesystem direct I/O paths support sub-page alignment.
                let sector_size = SECTOR_SIZE.max(BLOCK_SIZE) as i32;
                current_userspace!().write_val(raw_ioctl.arg(), &sector_size)?;
                Ok(0)
            }
            cmd @ HdIoGetGeo => {
                cmd.write(&self.hd_geometry())?;
                Ok(0)
            }
            cmd @ BlkGetSize => {
                let size = self.0.metadata().nr_sectors as u64;
                cmd.write(&size)?;
                Ok(0)
            }
            cmd @ BlkGetSize64 => {
                let size = (self.0.metadata().nr_sectors * SECTOR_SIZE) as u64;
                cmd.write(&size)?;
                Ok(0)
            }
            _cmd @ BlkRrPart => {
                self.reread_and_register_partitions();
                Ok(0)
            }
            _cmd @ BlkPg => {
                self.reread_and_register_partitions();
                Ok(0)
            }
            _cmd @ BlkFlsBuf => {
                Ok(0)
            }
            _ => return_errno_with_message!(
                Errno::ENOTTY,
                "the ioctl command is not supported by block devices"
            ),
        })
    }
}

impl OpenBlockFile {
    /// Collects legacy disk geometry for Linux-compatible tooling.
    fn hd_geometry(&self) -> HdGeometry {
        const HEADS: u8 = 255;
        const SECTORS: u8 = 63;

        let nr_sectors = u64::try_from(self.0.metadata().nr_sectors).unwrap_or(u64::MAX);
        let sectors_per_cylinder = u64::from(HEADS) * u64::from(SECTORS);
        let cylinders = nr_sectors / sectors_per_cylinder;
        let cylinders = cylinders.min(u64::from(u16::MAX)) as u16;

        HdGeometry {
            heads: HEADS,
            sectors: SECTORS,
            cylinders,
            _padding: 0,
            start: self.0.partition_start_sector().unwrap_or(0),
        }
    }

    /// Re-reads the partition table and registers any new partitions in devtmpfs.
    fn reread_and_register_partitions(&self) {
        aster_block::reread_partitions(&self.0);
        if let Some(partitions) = self.0.partitions() {
            let task = ostd::task::Task::current().unwrap();
            let thread_local = task.as_thread_local().unwrap();
            let fs_ref = thread_local.borrow_fs();
            let path_resolver = fs_ref.resolver().read();

            for partition in partitions {
                let block_file = Arc::new(BlockFile::new(partition));
                if let Some(devtmpfs_meta) = block_file.devtmpfs_meta() {
                    let dev_id = block_file.0.id().as_encoded_u64();
                    if let Err(e) =
                        add_node(DeviceType::Block, dev_id, &devtmpfs_meta, &path_resolver)
                        && e.error() != Errno::EEXIST
                    {
                        ostd::warn!("Failed to add partition node: {:?}", e);
                    }
                }
            }
        }
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
