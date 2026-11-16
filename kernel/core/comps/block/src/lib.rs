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
use ostd::sync::Mutex;

use self::{
    bio::{BioEnqueueError, SubmittedBio},
    prelude::*,
};
pub use self::{
    device_id::{MAX_MAJOR, MajorIdOwner, acquire_major, allocate_major},
    partition::PartitionManager,
};

pub const BLOCK_SIZE: usize = ostd::mm::PAGE_SIZE;
pub const SECTOR_SIZE: usize = 512;

/// The number of minor device numbers allocated for each whole-disk device,
/// including the whole disk and its partitions.
///
/// If a disk has more than 16 partitions, we will allocate device IDs for
/// remaining partitions via `EXTENDED_DEVICE_ID_ALLOCATOR`.
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

    /// Returns the partition manager of the block device.
    ///
    /// Whole-disk devices return their manager, which owns the device's
    /// partitions. Partition devices and devices without partition support
    /// return `None`.
    fn partition_manager(&self) -> Option<&PartitionManager> {
        None
    }
}

/// A block device whose queued requests are processed by a dedicated worker.
///
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

/// A function that starts a dedicated worker for a block request handler.
pub type RequestWorkerSpawner = fn(Arc<dyn BlockRequestHandler>);

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
    /// Request worker spawner registered
    RequestWorkerSpawnerRegistered,
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
pub fn register(device: Arc<dyn BlockDevice>) -> Result<(), Error> {
    register_entry(RegisteredDevice {
        device,
        request_handler: None,
    })
}

/// Registers a block device that requires a dedicated request worker.
///
/// If a request worker spawner is already registered, it is invoked for this device
/// before this function returns. Otherwise, the worker is started when the spawner
/// is registered.
pub fn register_with_request_handler(device: Arc<dyn BlockRequestHandler>) -> Result<(), Error> {
    register_entry(RegisteredDevice {
        device: device.clone(),
        request_handler: Some(device),
    })
}

/// Registers a device entry and starts its request worker if a spawner is available.
fn register_entry(entry: RegisteredDevice) -> Result<(), Error> {
    let request_worker = DEVICE_REGISTRY.lock().register(entry)?;
    if let Some((spawner, handler)) = request_worker {
        spawner(handler);
    }

    Ok(())
}

/// Registers the function used to start block request workers.
///
/// The spawner is invoked for every previously registered whole-disk request
/// handler before this function returns. Request handlers registered later invoke
/// the spawner as part of their registration.
///
/// # Errors
///
/// Returns [`Error::RequestWorkerSpawnerRegistered`] if a spawner is already registered.
pub fn register_request_worker_spawner(spawner: RequestWorkerSpawner) -> Result<(), Error> {
    let handlers = DEVICE_REGISTRY
        .lock()
        .register_request_worker_spawner(spawner)?;
    for handler in handlers {
        spawner(handler);
    }

    Ok(())
}

/// Unregisters an existing block device, returning the device if found.
///
/// This does not stop a request worker that has already been started for the device.
pub fn unregister(id: DeviceId) -> Result<Arc<dyn BlockDevice>, Error> {
    DEVICE_REGISTRY
        .lock()
        .devices
        .remove(&id.to_raw())
        .map(|entry| entry.device)
        .ok_or(Error::NotFound)
}

/// Collects all block devices.
pub fn collect_all() -> Vec<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY
        .lock()
        .devices
        .values()
        .map(|entry| entry.device.clone())
        .collect()
}

/// Looks up a block device of a given device ID.
pub fn lookup(id: DeviceId) -> Option<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY
        .lock()
        .devices
        .get(&id.to_raw())
        .map(|entry| entry.device.clone())
}

/// Looks up a block device by its kernel device name.
pub fn lookup_by_name(name: &str) -> Option<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY
        .lock()
        .devices
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

        let Some(partition_manager) = device.partition_manager() else {
            continue;
        };

        let Some(partition_infos) = partition::parse(&device) else {
            continue;
        };

        partition_manager.update(&device, partition_infos);
    }
}

struct RegisteredDevice {
    device: Arc<dyn BlockDevice>,
    request_handler: Option<Arc<dyn BlockRequestHandler>>,
}

type RequestWorker = (RequestWorkerSpawner, Arc<dyn BlockRequestHandler>);

struct DeviceRegistry {
    devices: BTreeMap<u32, RegisteredDevice>,
    request_worker_spawner: Option<RequestWorkerSpawner>,
}

impl DeviceRegistry {
    const fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
            request_worker_spawner: None,
        }
    }

    fn register(&mut self, entry: RegisteredDevice) -> Result<Option<RequestWorker>, Error> {
        let id = entry.device.id().to_raw();
        if self.devices.contains_key(&id) {
            return Err(Error::Registered);
        }

        let request_worker = if entry.device.is_partition() {
            None
        } else {
            self.request_worker_spawner.and_then(|spawner| {
                entry
                    .request_handler
                    .as_ref()
                    .map(|handler| (spawner, handler.clone()))
            })
        };
        self.devices.insert(id, entry);

        Ok(request_worker)
    }

    /// Registers the spawner and returns existing whole-disk handlers awaiting workers.
    fn register_request_worker_spawner(
        &mut self,
        spawner: RequestWorkerSpawner,
    ) -> Result<Vec<Arc<dyn BlockRequestHandler>>, Error> {
        if self.request_worker_spawner.is_some() {
            return Err(Error::RequestWorkerSpawnerRegistered);
        }
        self.request_worker_spawner = Some(spawner);

        Ok(self
            .devices
            .values()
            .filter(|entry| !entry.device.is_partition())
            .filter_map(|entry| entry.request_handler.clone())
            .collect())
    }
}

static DEVICE_REGISTRY: Mutex<DeviceRegistry> = Mutex::new(DeviceRegistry::new());

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    device_id::init();

    Ok(())
}
