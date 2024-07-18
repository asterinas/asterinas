// SPDX-License-Identifier: MPL-2.0

//! The input devices of Asterinas.
#![no_std]
#![deny(unsafe_code)]
#![feature(fn_traits)]

extern crate alloc;

pub mod key;

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::{any::Any, fmt::Debug};

use key::{Key, KeyStatus};
use ostd::{sync::SpinLock, ComponentInitError};
use spin::Once;

#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    KeyBoard(Key, KeyStatus),
}

pub trait InputDevice: Send + Sync + Any + Debug {
    fn register_callbacks(&self, function: &'static (dyn Fn(InputEvent) + Send + Sync));
}

pub fn register_device(name: String, device: Arc<dyn InputDevice>) {
    COMPONENT
        .get()
        .unwrap()
        .input_device_table
        .lock()
        .insert(name, device);
}

pub fn get_device(str: &str) -> Option<Arc<dyn InputDevice>> {
    COMPONENT
        .get()
        .unwrap()
        .input_device_table
        .lock()
        .get(str)
        .cloned()
}

pub fn all_devices() -> Vec<(String, Arc<dyn InputDevice>)> {
    let input_devs = COMPONENT.get().unwrap().input_device_table.lock();
    input_devs
        .iter()
        .map(|(name, device)| (name.clone(), device.clone()))
        .collect()
}

static COMPONENT: Once<Component> = Once::new();

#[ostd::init_comp]
fn component_init() -> Result<(), ComponentInitError> {
    let a = Component::init()?;
    COMPONENT.call_once(|| a);
    Ok(())
}

#[derive(Debug)]
struct Component {
    input_device_table: SpinLock<BTreeMap<String, Arc<dyn InputDevice>>>,
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            input_device_table: SpinLock::new(BTreeMap::new()),
        })
    }
}
