pub mod config;
pub mod device;
pub mod header;
pub mod control;
use alloc::sync::Arc;
use device::GPUDevice;
use ostd::sync::SpinLock;
use spin::Once;


pub static DEVICE_NAME: &str = "Virtio-GPU";
pub static GPU_DEVICE: Once<SpinLock<Arc<GPUDevice>>> = Once::new();