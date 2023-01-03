use jinux_frame::println;

use alloc::{sync::Arc, vec::Vec};
use jinux_frame::{TrapFrame, receive_char, info};
use lazy_static::lazy_static;
use spin::Mutex;

use crate::{process::Process, current};

lazy_static! {
    static ref KEYBOARD_CALLBACKS: Mutex<Vec<Arc<dyn Fn(u8) + Send + Sync + 'static>>> =
        Mutex::new(Vec::new());
    static ref WAIT_INPUT_PROCESS : Mutex<Option<Arc<Process>>> = Mutex::new(None);
}

pub fn init() {
    jinux_frame::device::console::register_console_input_callback(handle_irq);
    register_console_callback(Arc::new(console_receive_callback));
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

fn console_receive_callback(data: u8){
    let process = WAIT_INPUT_PROCESS.lock().take();
    process.unwrap().send_to_scheduler();
}

/// receive char from console, if there is no data in buffer, then it will switch to other task
/// until it is notified.
pub fn receive_console_char() -> u8{
    loop{
        if let Some(byte) = receive_char() {
            return byte;
        }else if WAIT_INPUT_PROCESS.lock().is_none(){
            WAIT_INPUT_PROCESS.lock().replace(current!()); 
            Process::yield_now();
            WAIT_INPUT_PROCESS.lock().take(); 
        }else{
            panic!("there is process waiting in the console receive list!");
        }
    }
}
