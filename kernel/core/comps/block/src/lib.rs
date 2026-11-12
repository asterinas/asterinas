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

use ::device_id::DeviceId;
use component::{ComponentInitError, init_component};
use ostd::sync::Mutex;

pub use self::device_id::{MAX_MAJOR, MajorIdOwner, acquire_major, allocate_major};
use self::{
    bio::{BioEnqueueError, SubmittedBio},
    prelude::*,
};

pub const BLOCK_SIZE: usize = ostd::mm::PAGE_SIZE;
pub const SECTOR_SIZE: usize = 512;

/// The number of minor device numbers allocated for each whole-disk device,
/// including the whole disk and its partitions.
///
/// If a disk has more than 16 partitions, we will allocate device IDs for
/// remaining partitions via `EXTENDED_DEVICE_ID_ALLOCATOR`.
pub const DEVICE_MINORS: u32 = 16;

pub trait AnyBlockDevice: Send + Sync + Any + Debug {
    /// Enqueues a new `SubmittedBio` to the block device.
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError>;

    /// Returns the metadata of the block device.
    fn metadata(&self) -> BlockDeviceMeta;

    /// Returns the name of the block device.
    fn name(&self) -> &str;

    /// Returns the device ID of the block device.
    fn id(&self) -> DeviceId;

    /// Returns the kind of the block device.
    fn kind(&self) -> BlockDeviceKind<'_>;
}

/// A whole disk with request processing.
pub trait WholeDisk: AnyBlockDevice {
    /// Handles one request, waiting if the queue is empty.
    ///
    /// Returns `Some(())` to continue processing, or `None` if no background
    /// request processing is needed.
    /// The I/O result is reported through BIO completion.
    fn handle_requests(&self) -> Option<()>;
}

/// The kind of a block device.
pub enum BlockDeviceKind<'a> {
    /// A whole disk.
    Disk(&'a dyn WholeDisk),
    /// A partition of a whole disk.
    Partition {
        /// Partition number, starting at one.
        number: u32,
        /// Start offset in units of 512-byte sectors on the whole disk.
        start_sector: u64,
    },
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

impl dyn AnyBlockDevice {
    pub fn downcast_ref<T: AnyBlockDevice>(&self) -> Option<&T> {
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
    /// ID acquired
    IdAcquired,
    /// ID exhausted
    IdExhausted,
}

/// Registers a new block device.
pub fn register(device: Arc<dyn AnyBlockDevice>) -> Result<(), Error> {
    let mut registry = DEVICE_REGISTRY.lock();
    let id = device.id().to_raw();
    if registry.contains_key(&id) {
        return Err(Error::Registered);
    }
    registry.insert(id, device);

    Ok(())
}

/// Unregisters an existing block device, returning the device if found.
pub fn unregister(id: DeviceId) -> Result<Arc<dyn AnyBlockDevice>, Error> {
    DEVICE_REGISTRY
        .lock()
        .remove(&id.to_raw())
        .ok_or(Error::NotFound)
}

/// Collects all block devices.
pub fn collect_all() -> Vec<Arc<dyn AnyBlockDevice>> {
    DEVICE_REGISTRY.lock().values().cloned().collect()
}

/// Looks up a block device of a given device ID.
pub fn lookup(id: DeviceId) -> Option<Arc<dyn AnyBlockDevice>> {
    DEVICE_REGISTRY.lock().get(&id.to_raw()).cloned()
}

/// Looks up a block device by its kernel device name.
pub fn lookup_by_name(name: &str) -> Option<Arc<dyn AnyBlockDevice>> {
    DEVICE_REGISTRY
        .lock()
        .values()
        .find(|device| device.name() == name)
        .cloned()
}

/// Scans registered whole disks for their initial partitions.
pub fn scan_partitions() {
    let devices = collect_all();
    for device in devices {
        if !matches!(device.kind(), BlockDeviceKind::Disk(_)) {
            continue;
        }

        partition::scan(&device);
    }
}

static DEVICE_REGISTRY: Mutex<BTreeMap<u32, Arc<dyn AnyBlockDevice>>> = Mutex::new(BTreeMap::new());

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    device_id::init();

    Ok(())
}
