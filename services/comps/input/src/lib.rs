//! The input device of jinux
#![no_std]
#![forbid(unsafe_code)]
#![feature(fn_traits)]

mod virtio;

extern crate alloc;
use core::any::Any;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use component::init_component;
use component::ComponentInitError;
use jinux_virtio::VirtioDeviceType;

use spin::{Mutex, Once};
use virtio::VirtioInputDevice;
use virtio_input_decoder::DecodeType;

pub trait INPUTDevice: Send + Sync + Any {
    fn handle_irq(&self) -> Option<()>;
    fn register_callbacks(&self, function: &'static (dyn Fn(DecodeType) + Send + Sync));
    fn name(&self) -> &String;
}

pub static INPUT_COMPONENT: Once<INPUTComponent> = Once::new();

#[init_component]
fn input_component_init() -> Result<(), ComponentInitError> {
    let a = INPUTComponent::init()?;
    INPUT_COMPONENT.call_once(|| a);
    Ok(())
}

pub struct INPUTComponent {
    /// Input device map, key is the irq number, value is the Input device
    input_device_map: Mutex<BTreeMap<u8, Arc<dyn INPUTDevice>>>,
}

impl INPUTComponent {
    pub fn init() -> Result<Self, ComponentInitError> {
        let mut input_device_map: BTreeMap<u8, Arc<dyn INPUTDevice>> = BTreeMap::new();
        let virtio = jinux_virtio::VIRTIO_COMPONENT.get().unwrap();
        let devices = virtio.get_device(VirtioDeviceType::Input);
        for device in devices {
            let (v_device, irq_num) = VirtioInputDevice::new(device);
            input_device_map.insert(irq_num, Arc::new(v_device));
        }
        Ok(Self {
            input_device_map: Mutex::new(input_device_map),
        })
    }

    pub const fn name() -> &'static str {
        "Input Device"
    }
    // 0~65535
    pub const fn priority() -> u16 {
        8192
    }
}

impl INPUTComponent {
    fn call(self: &Self, irq_number: u8) -> Result<(), InputDeviceHandleError> {
        // FIXME: use Result instead
        let binding = self.input_device_map.lock();
        let device = binding
            .get(&irq_number)
            .ok_or(InputDeviceHandleError::DeviceNotExists)?;
        device.handle_irq();
        Ok(())
    }

    pub fn get_input_device(self: &Self) -> Vec<Arc<dyn INPUTDevice>> {
        self.input_device_map
            .lock()
            .iter()
            .map(|(_, device)| device.clone())
            .collect::<Vec<Arc<dyn INPUTDevice>>>()
    }
}

#[derive(Debug)]
enum InputDeviceHandleError {
    DeviceNotExists,
    Unknown,
}
