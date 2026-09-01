// SPDX-License-Identifier: MPL-2.0

//! The block devices of Asterinas.
//！
//！This crate provides a number of base components for block devices, including
//! an abstraction of block devices, as well as the registration and lookup of block devices.
//!
//! Block devices use a queue-based model for asynchronous I/O operations. It is necessary
//! for a block device to maintain a queue to handle I/O requests. The users (e.g., fs)
//! submit I/O requests to this queue and wait for their completion. Drivers implementing
//! block devices can create their own queues as needed, with the possibility to reorder
//! and merge requests within the queue.
//!
//! This crate also offers the `Bio` related data structures and APIs to accomplish
//! safe and convenient block I/O operations, for example:
//!
//! ```no_run
//! // Creates a bio request.
//! let bio = Bio::new(BioType::Write, sid, segments, None);
//! // Submits to the block device.
//! let mut io_batch = IoBatch::new();
//! bio.submit(block_device, &mut io_batch)?;
//! // Waits for the the completion.
//! io_batch.wait_all()?;
//! ```
//!
#![no_std]
#![deny(unsafe_code)]
#![feature(step_trait)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "block: "
    };
}

pub mod bio;
mod device_id;
pub mod id;
mod impl_block_device;
mod partition;
mod prelude;
pub mod request_queue;

use alloc::format;

use ::device_id::{DeviceId, MinorId};
use component::{ComponentInitError, init_component};
pub use device_id::{EXTENDED_DEVICE_ID_ALLOCATOR, MajorIdOwner, acquire_major, allocate_major};
use ostd::sync::Mutex;
pub use partition::{PartitionInfo, PartitionNode};

use self::{
    bio::{BioEnqueueError, SubmittedBio},
    prelude::*,
};

pub const BLOCK_SIZE: usize = ostd::mm::PAGE_SIZE;
pub const SECTOR_SIZE: usize = 512;

/// The number of minor device numbers allocated for each whole-disk device,
/// including the whole disk and its partitions. If a disk has more than
/// 16 partitions, then allocate a device ID via `EXTENDED_DEVICE_ID_ALLOCATOR`.
pub const DEVICE_MINORS: u32 = 16;

pub trait BlockDevice: Send + Sync + Any + Debug {
    /// Enqueues a new `SubmittedBio` to the block device.
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError>;

    /// Returns the metadata of the block device.
    fn metadata(&self) -> BlockDeviceMeta;

    /// Returns the name of the block device.
    fn name(&self) -> &str;

    /// Returns the device ID of the block device.
    fn id(&self) -> DeviceId;

    /// Returns whether the block device is a partition.
    fn is_partition(&self) -> bool {
        false
    }

    /// Sets the partitions of the block device.
    fn set_partitions(&self, _partitions: Vec<Arc<PartitionNode>>) {}

    /// Returns the partitions of the block device.
    fn partitions(&self) -> Option<Vec<Arc<dyn BlockDevice>>> {
        None
    }

    /// Updates the partitions of the block device with the parsed partition
    /// information
    fn update_partitions(&self, infos: Vec<Option<PartitionInfo>>) {
        let Some(device) = lookup(self.id()) else {
            return;
        };

        if let Some(old_partitions) = self.partitions() {
            for partition in old_partitions {
                let _ = unregister(partition.id());
            }
        }

        let mut new_partitions = Vec::new();
        for (index, info_opt) in infos.iter().enumerate() {
            let Some(info) = info_opt else {
                continue;
            };

            let index = index as u32 + 1;
            let id = if index < DEVICE_MINORS {
                DeviceId::new(
                    self.id().major(),
                    MinorId::new(self.id().minor().get() + index),
                )
            } else {
                EXTENDED_DEVICE_ID_ALLOCATOR.get().unwrap().allocate()
            };
            let name = partition_name(self.name(), index);
            let partition = Arc::new(PartitionNode::new(id, name, device.clone(), *info));
            new_partitions.push(partition);
        }

        for partition in new_partitions.iter() {
            let _ = register(partition.clone());
        }

        self.set_partitions(new_partitions);
    }
}

/// Formats the name of a partition. We perform the naming similar to the
/// Linux implementation: insert "p" between the disk name and the partition
/// number when the disk name ends with a digit (`nvme0n1p1`), and append the
/// number otherwise (`vda1`).
/// Reference: <https://elixir.bootlin.com/linux/v7.2.2/source/block/partitions/core.c#L337>
fn partition_name(disk_name: &str, partno: u32) -> String {
    if disk_name.ends_with(|c: char| c.is_ascii_digit()) {
        format!("{}p{}", disk_name, partno)
    } else {
        format!("{}{}", disk_name, partno)
    }
}

/// Metadata for a block device.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockDeviceMeta {
    /// The upper limit for the number of segments per bio.
    pub max_nr_segments_per_bio: usize,
    /// The total number of sectors of the block device.
    pub nr_sectors: usize,
    // Additional useful metadata can be added here in the future.
}

impl dyn BlockDevice {
    pub fn downcast_ref<T: BlockDevice>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}

/// The error type which is returned from the APIs of this crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Device registered
    Registered,
    /// Device not found
    NotFound,
    /// Invalid arguments
    InvalidArgs,
    /// Id Acquired
    IdAcquired,
    /// Id Exhausted
    IdExhausted,
}

/// Registers a new block device.
pub fn register(device: Arc<dyn BlockDevice>) -> Result<(), Error> {
    let mut registry = DEVICE_REGISTRY.lock();
    let id = device.id().to_raw();
    if registry.contains_key(&id) {
        return Err(Error::Registered);
    }
    registry.insert(id, device);

    Ok(())
}

/// Unregisters an existing block device, returning the device if found.
pub fn unregister(id: DeviceId) -> Result<Arc<dyn BlockDevice>, Error> {
    DEVICE_REGISTRY
        .lock()
        .remove(&id.to_raw())
        .ok_or(Error::NotFound)
}

/// Collects all block devices.
pub fn collect_all() -> Vec<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY.lock().values().cloned().collect()
}

/// Looks up a block device of a given device ID.
pub fn lookup(id: DeviceId) -> Option<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY.lock().get(&id.to_raw()).cloned()
}

/// Looks up a block device by its kernel device name.
pub fn lookup_by_name(name: &str) -> Option<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY
        .lock()
        .values()
        .find(|device| device.name() == name)
        .cloned()
}

/// Scans registered whole-disk devices and updates their partitions.
pub fn scan_partitions() {
    let devices = collect_all();
    for device in devices {
        if device.is_partition() {
            continue;
        }

        let Some(partition_info) = partition::parse(&device) else {
            continue;
        };

        device.update_partitions(partition_info);
    }
}

static DEVICE_REGISTRY: Mutex<BTreeMap<u32, Arc<dyn BlockDevice>>> = Mutex::new(BTreeMap::new());

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    device_id::init();

    Ok(())
}
