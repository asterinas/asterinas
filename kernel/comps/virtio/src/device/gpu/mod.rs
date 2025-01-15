use alloc::sync::Arc;

use device::GPUDevice;
use ostd::sync::SpinLock;
use spin::Once;

pub mod config;
pub mod control;
pub mod device;
pub mod header;

pub static DEVICE_NAME: &str = "Virtio-GPU";

// global static variable
pub static GPU_DEVICE: Once<SpinLock<Arc<GPUDevice>>> = Once::new();
