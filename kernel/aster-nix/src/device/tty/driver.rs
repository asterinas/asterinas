// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use ostd::mm::{Infallible, VmReader};
use spin::Once;

use crate::{
    device::tty::{get_n_tty, Tty},
    prelude::*,
};

pub static TTY_DRIVER: Once<Arc<TtyDriver>> = Once::new();

pub(super) fn init() {
    for (_, device) in aster_console::all_devices() {
        device.register_callback(&console_input_callback)
    }
    let tty_driver = Arc::new(TtyDriver::new());
    // FIXME: install n_tty into tty_driver?
    let n_tty = get_n_tty();
    tty_driver.install(n_tty.clone());
    TTY_DRIVER.call_once(|| tty_driver);
}

pub struct TtyDriver {
    ttys: SpinLock<Vec<Arc<Tty>>>,
}

impl TtyDriver {
    pub const fn new() -> Self {
        Self {
            ttys: SpinLock::new(Vec::new()),
        }
    }

    /// Return the tty device in driver's internal table.
    pub fn lookup(&self, index: usize) -> Result<Arc<Tty>> {
        let ttys = self.ttys.disable_irq().lock();
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
        self.ttys.disable_irq().lock().push(tty);
    }

    /// remove a new tty into the driver's internal tables.
    pub fn remove(&self, index: usize) -> Result<()> {
        let mut ttys = self.ttys.disable_irq().lock();
        if index >= ttys.len() {
            return_errno_with_message!(Errno::ENODEV, "lookup failed. No tty device");
        }
        let removed_tty = ttys.remove(index);
        removed_tty.set_driver(Weak::new());
        drop(ttys);
        Ok(())
    }

    pub fn push_char(&self, ch: u8) {
        // FIXME: should the char send to all ttys?
        for tty in &*self.ttys.disable_irq().lock() {
            tty.push_char(ch);
        }
    }
}

impl Default for TtyDriver {
    fn default() -> Self {
        Self::new()
    }
}

fn console_input_callback(mut reader: VmReader<Infallible>) {
    let tty_driver = get_tty_driver();
    while reader.remain() > 0 {
        let ch = reader.read_val().unwrap();
        tty_driver.push_char(ch);
    }
}

fn serial_input_callback(item: u8) {
    let tty_driver = get_tty_driver();
    tty_driver.push_char(item);
}

fn get_tty_driver() -> &'static TtyDriver {
    TTY_DRIVER.get().unwrap()
}
