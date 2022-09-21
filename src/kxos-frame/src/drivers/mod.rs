pub(crate) mod msix;
mod pci;
pub mod virtio;

pub fn init() {
    pci::init();
}
