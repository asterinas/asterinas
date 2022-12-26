pub mod console;
pub mod pci;

pub fn init() {
    pci::init();
    console::init();
}
