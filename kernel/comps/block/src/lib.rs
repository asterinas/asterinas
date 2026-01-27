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
//! let bio_waiter = bio.submit(block_device)?;
//! // Waits for the the completion.
//! let Some(status) = bio_waiter.wait() else {
//!     return Err(IoError);
//! };
//! assert!(status == BioStatus::Complete);
//! ```
//!
#![no_std]
#![deny(unsafe_code)]
#![feature(step_trait)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

pub mod bio;
mod device_id;
pub mod id;
mod impl_block_device;
mod partition;
mod prelude;
pub mod request_queue;

use ::device_id::DeviceId;
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
    fn set_partitions(&self, _infos: Vec<Option<PartitionInfo>>) {}

    /// Returns the partitions of the block device.
    fn partitions(&self) -> Option<Vec<Arc<dyn BlockDevice>>> {
        None
    }
}

/// Metadata for a block device.
#[derive(Debug, Default, Clone, Copy)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

static DEVICE_REGISTRY: Mutex<BTreeMap<u32, Arc<dyn BlockDevice>>> = Mutex::new(BTreeMap::new());

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    device_id::init();

    Ok(())
}

#[init_component(process)]
fn init_in_first_process() -> Result<(), component::ComponentInitError> {
    let devices = collect_all();
    for device in devices {
        let Some(partition_info) = partition::parse(&device) else {
            continue;
        };

        device.set_partitions(partition_info);
    }

    Ok(())
}
