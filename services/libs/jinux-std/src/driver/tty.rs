pub use jinux_frame::arch::x86::device::serial::register_serial_input_callback;

use crate::{
    prelude::*,
    tty::{get_n_tty, Tty},
};

lazy_static! {
    pub static ref TTY_DRIVER: Arc<TtyDriver> = {
        let tty_driver = Arc::new(TtyDriver::new());
        // FIXME: install n_tty into tty_driver?
        let n_tty = get_n_tty();
        tty_driver.install(n_tty.clone());
        tty_driver
    };
}

pub struct TtyDriver {
    ttys: Mutex<Vec<Arc<Tty>>>,
}

impl TtyDriver {
    pub fn new() -> Self {
        Self {
            ttys: Mutex::new(Vec::new()),
        }
    }

    /// Return the tty device in driver's internal table.
    pub fn lookup(&self, index: usize) -> Result<Arc<Tty>> {
        let ttys = self.ttys.lock();
        // Return the tty device corresponding to idx
        if index >= ttys.len() {
            return_errno_with_message!(Errno::ENODEV, "lookup failed. No tty device");
        }
        let tty = ttys[index].clone();
        drop(ttys);
        Ok(tty)
    }

    /// Install a new tty into the driver's internal tables.
    pub fn install(self: &Arc<Self>, tty: Arc<Tty>) {
        tty.set_driver(Arc::downgrade(self));
        self.ttys.lock().push(tty);
    }

    /// remove a new tty into the driver's internal tables.
    pub fn remove(&self, index: usize) -> Result<()> {
        let mut ttys = self.ttys.lock();
        if index >= ttys.len() {
            return_errno_with_message!(Errno::ENODEV, "lookup failed. No tty device");
        }
        let removed_tty = ttys.remove(index);
        removed_tty.set_driver(Weak::new());
        drop(ttys);
        Ok(())
    }

    pub fn receive_char(&self, item: u8) {
        // FIXME: should the char send to all ttys?
        for tty in &*self.ttys.lock() {
            tty.receive_char(item);
        }
    }
}

fn serial_input_callback(item: u8) {
    let tty_driver = get_tty_driver();
    tty_driver.receive_char(item);
}

fn get_tty_driver() -> &'static TtyDriver {
    &TTY_DRIVER
}

pub fn init() {
    register_serial_input_callback(serial_input_callback);
}
