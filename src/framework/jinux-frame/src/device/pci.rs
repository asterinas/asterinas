//! PCI bus io port

use super::io_port::IoPort;
use lazy_static::lazy_static;

const CONFIG_ADDRESS: u16 = 0x0CF8;
const CONFIG_DATA: u16 = 0x0CFC;

lazy_static! {
    pub static ref PCI_ADDRESS_PORT: IoPort = unsafe { IoPort::new(CONFIG_ADDRESS).unwrap() };
    pub static ref PCI_DATA_PORT: IoPort = unsafe { IoPort::new(CONFIG_DATA).unwrap() };
}
