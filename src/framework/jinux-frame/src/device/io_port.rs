use spin::Mutex;
use x86_64::{
    instructions::port::Port,
    structures::port::{PortRead, PortWrite},
};

/// An I/O port, representing a specific address in the I/O address of x86.
///
/// The following code shows and example to read and write u32 value to an I/O port:
///
/// ```rust
/// static PORT: IoPort<u32> = IoPort::new(0x12);
///
/// fn port_value_increase(){
///     PORT.write(PORT.read() + 1)
/// }
/// ```
///
pub struct IoPort<T> {
    addr: u16,
    port: Mutex<Port<T>>,
}

impl<T> IoPort<T> {
    /// Create an I/O port.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as creating an I/O port is considered
    /// a privileged operation.
    pub const unsafe fn new(addr: u16) -> Self {
        Self {
            addr,
            port: Mutex::new(Port::new(addr)),
        }
    }
}

impl<T> IoPort<T> {
    /// Get the address of this I/O port.
    pub fn addr(&self) -> u16 {
        self.addr
    }
}

impl<T: PortRead> IoPort<T> {
    /// Reads from the port.
    pub fn read(&self) -> T {
        unsafe { (*self.port.lock()).read() }
    }
}

impl<T: PortWrite> IoPort<T> {
    /// Writes to the port.
    pub fn write(&self, val: T) {
        unsafe {
            self.port.lock().write(val);
        }
    }
}
