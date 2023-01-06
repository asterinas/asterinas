pub mod pci;
pub mod tty;

pub fn init() {
    pci::init();
    tty::init();
}
