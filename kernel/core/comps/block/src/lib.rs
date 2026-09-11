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

#[cfg(ktest)]
mod tests;

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

/// A block device whose queued requests are processed by a dedicated worker.
///
/// Registration alone does not start a worker; see [`register_with_request_handler`].
/// Devices that process or forward requests without such a worker only need to
/// implement [`BlockDevice`].
pub trait BlockRequestHandler: BlockDevice {
    /// Waits for and processes the next queued request.
    ///
    /// This method may sleep. The caller must use a sleepable thread context and
    /// must not call this method concurrently on the same handler. Returning does
    /// not imply I/O completion. I/O errors are reported through the submitted
    /// BIO's completion status.
    fn handle_next_request(&self);
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
    register_entry(RegisteredDevice {
        device,
        request_handler: None,
    })
}

/// Registers a block device that requires a dedicated request worker.
///
/// Registration does not start the worker. The kernel starts workers for registered
/// handlers once during boot, before scanning partitions. The device must therefore
/// be registered before that startup pass. Later registration does not start a worker.
pub fn register_with_request_handler<T: BlockRequestHandler>(device: Arc<T>) -> Result<(), Error> {
    register_entry(RegisteredDevice {
        device: device.clone(),
        request_handler: Some(device),
    })
}

fn register_entry(entry: RegisteredDevice) -> Result<(), Error> {
    let mut registry = DEVICE_REGISTRY.lock();
    let id = entry.device.id().to_raw();
    if registry.contains_key(&id) {
        return Err(Error::Registered);
    }
    registry.insert(id, entry);

    Ok(())
}

/// Unregisters an existing block device, returning the device if found.
///
/// This does not stop a request worker that has already been started for the device.
pub fn unregister(id: DeviceId) -> Result<Arc<dyn BlockDevice>, Error> {
    DEVICE_REGISTRY
        .lock()
        .remove(&id.to_raw())
        .map(|entry| entry.device)
        .ok_or(Error::NotFound)
}

/// Collects all block devices.
pub fn collect_all() -> Vec<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY
        .lock()
        .values()
        .map(|entry| entry.device.clone())
        .collect()
}

/// Collects the request handlers of registered whole-disk devices in device ID order.
///
/// The returned snapshot does not hold the registry lock. Collecting handlers does
/// not consume them; the kernel must start their workers only once and before any
/// synchronous I/O that needs those workers, including partition scanning.
pub fn collect_request_handlers() -> Vec<Arc<dyn BlockRequestHandler>> {
    let mut handlers: Vec<_> = DEVICE_REGISTRY
        .lock()
        .values()
        .filter_map(|entry| entry.request_handler.clone())
        .collect();
    handlers.retain(|handler| !handler.is_partition());
    handlers
}

/// Looks up a block device of a given device ID.
pub fn lookup(id: DeviceId) -> Option<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY
        .lock()
        .get(&id.to_raw())
        .map(|entry| entry.device.clone())
}

/// Looks up a block device by its kernel device name.
pub fn lookup_by_name(name: &str) -> Option<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY
        .lock()
        .values()
        .find(|entry| entry.device.name() == name)
        .map(|entry| entry.device.clone())
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

        device.set_partitions(partition_info);
    }
}

struct RegisteredDevice {
    device: Arc<dyn BlockDevice>,
    request_handler: Option<Arc<dyn BlockRequestHandler>>,
}

static DEVICE_REGISTRY: Mutex<BTreeMap<u32, RegisteredDevice>> = Mutex::new(BTreeMap::new());

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    device_id::init();

    Ok(())
}
