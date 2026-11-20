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
mod class;
mod device_id;
pub mod id;
mod impl_block_device;
mod partition;
mod prelude;
pub mod request_queue;

use ::device_id::DeviceId;
use aster_device::common::{AnyDevice, DevNum};
use component::{ComponentInitError, init_component};

use self::{
    bio::{BioEnqueueError, SubmittedBio},
    prelude::*,
};
pub use self::{
    class::{BLOCK_CLASS, BlockClass, BlockDevice, DISK_TYPE},
    device_id::{MAX_MAJOR, MajorIdOwner, acquire_major, allocate_major},
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
#[derive(Debug)]
pub enum Error {
    /// Invalid arguments
    InvalidArgs,
    /// ID acquired
    IdAcquired,
    /// ID exhausted
    IdExhausted,
    /// Block device I/O failed.
    IoError,
    /// Device registration or removal failed.
    DeviceModel(aster_device::common::Error),
}

/// Collects all block devices.
pub fn collect_all() -> Vec<Arc<BlockDevice>> {
    BLOCK_CLASS.get().unwrap().devices()
}

/// Looks up a block device of a given device ID.
pub fn lookup(id: DeviceId) -> Option<Arc<BlockDevice>> {
    collect_all()
        .into_iter()
        .find(|device| device.devnum() == Some(DevNum::block(id)))
}

/// Looks up a block device by its kernel device name.
pub fn lookup_by_name(name: &str) -> Option<Arc<BlockDevice>> {
    BLOCK_CLASS.get().unwrap().find_device(name)
}

/// Scans registered whole disks for their initial partitions.
pub fn scan_partitions() {
    let devices = collect_all();
    for device in devices {
        if !matches!(device.kind(), BlockDeviceKind::Disk(_)) {
            continue;
        }

        if let Err(err) = partition::scan(&device) {
            ostd::error!("cannot scan partitions of {}: {:?}", device.name(), err);
        }
    }
}

/// Initializes the block class for kernel-mode tests.
#[cfg(ktest)]
pub fn init_for_ktest() {
    aster_device::init_for_ktest();
    init().unwrap();
}

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    device_id::init();
    BLOCK_CLASS.call_once(|| aster_device::class::register(BlockClass).unwrap());

    Ok(())
}
