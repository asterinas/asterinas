//! The console device of Asterinas.
#![no_std]
#![forbid(unsafe_code)]
#![feature(fn_traits)]

extern crate alloc;

use alloc::{collections::BTreeMap, fmt::Debug, string::String, sync::Arc, vec::Vec};
use core::any::Any;

use aster_frame::sync::SpinLock;
use component::{init_component, ComponentInitError};
use spin::Once;

pub type ConsoleCallback = dyn Fn(&[u8]) + Send + Sync;

pub trait AnyConsoleDevice: Send + Sync + Any + Debug {
    fn send(&self, buf: &[u8]);
    fn recv(&self, buf: &mut [u8]) -> Option<usize>;
    fn register_callback(&self, callback: &'static ConsoleCallback);
    fn handle_irq(&self);
}

pub fn register_device(name: String, device: Arc<dyn AnyConsoleDevice>) {
    COMPONENT
        .get()
        .unwrap()
        .console_device_table
        .lock()
        .insert(name, device);
}

pub fn get_device(str: &str) -> Option<Arc<dyn AnyConsoleDevice>> {
    COMPONENT
        .get()
        .unwrap()
        .console_device_table
        .lock()
        .get(str)
        .cloned()
}

pub fn all_devices() -> Vec<(String, Arc<dyn AnyConsoleDevice>)> {
    let console_devs = COMPONENT.get().unwrap().console_device_table.lock();
    console_devs
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
    console_device_table: SpinLock<BTreeMap<String, Arc<dyn AnyConsoleDevice>>>,
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            console_device_table: SpinLock::new(BTreeMap::new()),
        })
    }
}
