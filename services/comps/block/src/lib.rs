//! The block devices of jinux
#![no_std]
#![forbid(unsafe_code)]
#![feature(fn_traits)]

extern crate alloc;

use core::any::Any;
use core::fmt::Debug;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use component::init_component;
use component::ComponentInitError;
use jinux_frame::sync::SpinLock;
use spin::Once;

pub const BLK_SIZE: usize = 512;

pub trait BlockDevice: Send + Sync + Any + Debug {
    fn read_block(&self, block_id: usize, buf: &mut [u8]);
    fn write_block(&self, block_id: usize, buf: &[u8]);
    fn handle_irq(&self);
}

pub fn register_device(name: String, device: Arc<dyn BlockDevice>) {
    COMPONENT.get().unwrap().devices.lock().insert(name, device);
}

pub fn get_device(str: &String) -> Option<Arc<dyn BlockDevice>> {
    COMPONENT.get().unwrap().devices.lock().get(str).cloned()
}

pub fn all_devices() -> Vec<(String, Arc<dyn BlockDevice>)> {
    let lock = COMPONENT.get().unwrap().devices.lock();
    let mut vec = Vec::new();
    for (name, device) in lock.iter() {
        vec.push((name.clone(), device.clone()));
    }
    vec
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
    devices: SpinLock<BTreeMap<String, Arc<dyn BlockDevice>>>,
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            devices: SpinLock::new(BTreeMap::new()),
        })
    }
}
