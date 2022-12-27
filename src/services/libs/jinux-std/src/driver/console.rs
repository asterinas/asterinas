use alloc::{sync::Arc, vec::Vec};
use jinux_frame::TrapFrame;
use lazy_static::lazy_static;
use spin::Mutex;

lazy_static! {
    static ref KEYBOARD_CALLBACKS: Mutex<Vec<Arc<dyn Fn(u8) + Send + Sync + 'static>>> =
        Mutex::new(Vec::new());
}

pub fn init() {
    jinux_frame::device::console::register_console_input_callback(handle_irq)
}

fn handle_irq(trap_frame: &TrapFrame) {
    if KEYBOARD_CALLBACKS.is_locked() {
        return;
    }
    let lock = KEYBOARD_CALLBACKS.lock();
    for callback in lock.iter() {
        callback.call(((jinux_frame::device::console::receive_char().unwrap()),));
    }
}

pub fn register_console_callback(callback: Arc<dyn Fn(u8) + 'static + Send + Sync>) {
    KEYBOARD_CALLBACKS.lock().push(callback);
}
