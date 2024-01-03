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
//! safe and convenient block I/O operations, for exmaple:
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
#![forbid(unsafe_code)]
#![feature(fn_traits)]
#![feature(step_trait)]
#![allow(dead_code)]

extern crate alloc;

pub mod bio;
pub mod id;
mod impl_block_device;
mod prelude;
pub mod request_queue;

use self::{prelude::*, request_queue::BioRequestQueue};

use aster_frame::sync::SpinLock;
use component::init_component;
use component::ComponentInitError;

use spin::Once;

pub const BLOCK_SIZE: usize = aster_frame::config::PAGE_SIZE;
pub const SECTOR_SIZE: usize = 512;

pub trait BlockDevice: Send + Sync + Any + Debug {
    /// Returns this block device's request queue, to which block I/O requests may be submitted.
    fn request_queue(&self) -> &dyn BioRequestQueue;
    fn handle_irq(&self);
}

impl dyn BlockDevice {
    pub fn downcast_ref<T: BlockDevice>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}

pub fn register_device(name: String, device: Arc<dyn BlockDevice>) {
    COMPONENT
        .get()
        .unwrap()
        .block_device_table
        .lock()
        .insert(name, device);
}

pub fn get_device(str: &str) -> Option<Arc<dyn BlockDevice>> {
    COMPONENT
        .get()
        .unwrap()
        .block_device_table
        .lock()
        .get(str)
        .cloned()
}

pub fn all_devices() -> Vec<(String, Arc<dyn BlockDevice>)> {
    let block_devs = COMPONENT.get().unwrap().block_device_table.lock();
    block_devs
        .iter()
        .map(|(name, device)| (name.clone(), device.clone()))
        .collect()
}

static COMPONENT: Once<Component> = Once::new();

#[init_component]
fn component_init() -> Result<(), ComponentInitError> {
    let a = Component::init()?;
    COMPONENT.call_once(|| a);
    Ok(())
}

#[derive(Debug)]
struct Component {
    block_device_table: SpinLock<BTreeMap<String, Arc<dyn BlockDevice>>>,
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            block_device_table: SpinLock::new(BTreeMap::new()),
        })
    }
}
