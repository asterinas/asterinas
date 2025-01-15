pub mod config;
pub mod device;
pub mod header;
pub mod control;
use device::GPUDevice;

pub static DEVICE_NAME: &str = "Virtio-GPU";
pub static GPU_DEVICE: Once<SpinLock<Arc<GPUDevice>>> = Once::new();