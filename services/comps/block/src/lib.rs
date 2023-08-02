//! The block device of jinux
#![no_std]
#![forbid(unsafe_code)]
#![feature(fn_traits)]

mod virtio;

extern crate alloc;

use core::any::Any;

use alloc::string::ToString;
use alloc::sync::Arc;
use component::init_component;
use component::ComponentInitError;
use jinux_virtio::VirtioDeviceType;
use spin::Once;
use virtio::VirtioBlockDevice;

pub const BLK_SIZE: usize = 512;

pub trait BlockDevice: Send + Sync + Any {
    fn init(&self) {}
    fn read_block(&self, block_id: usize, buf: &mut [u8]);
    fn write_block(&self, block_id: usize, buf: &[u8]);
    fn handle_irq(&self);
}

pub static BLK_COMPONENT: Once<BLKComponent> = Once::new();

#[init_component]
fn blk_component_init() -> Result<(), ComponentInitError> {
    let a = BLKComponent::init()?;
    BLK_COMPONENT.call_once(|| a);
    Ok(())
}

pub struct BLKComponent {
    /// Input device map, key is the irq number, value is the Input device
    blk_device: Arc<dyn BlockDevice>,
}

impl BLKComponent {
    pub fn init() -> Result<Self, ComponentInitError> {
        let virtio = jinux_virtio::VIRTIO_COMPONENT.get().unwrap();
        let devices = virtio.get_device(VirtioDeviceType::Block);
        // FIXME: deal with multiple block devices
        if let Some(device) = devices.into_iter().next() {
            let v_device = VirtioBlockDevice::new(device);
            return Ok(Self {
                blk_device: Arc::new(v_device),
            });
        }
        Err(ComponentInitError::UninitializedDependencies(
            "Virtio".to_string(),
        ))
    }

    pub const fn name() -> &'static str {
        "Block device"
    }
    // 0~65535
    pub const fn priority() -> u16 {
        8192
    }
}

impl BLKComponent {
    pub fn get_device(self: &Self) -> Arc<dyn BlockDevice> {
        self.blk_device.clone()
    }
}
