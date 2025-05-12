// SPDX-License-Identifier: MPL-2.0

//! The i8042 mouse driver.

use core::sync::atomic::{AtomicBool, Ordering};
use ostd::{
    arch::{device::io_port::ReadWriteAccess, IO_APIC},
    io::IoPort,
    sync::SpinLock,
    trap::{IrqLine, TrapFrame},
    sync::Mutex,
};
use spin::Once;
use alloc::sync::Arc;
use aster_input::{InputDevice, InputDeviceMeta, InputEvent, input_event};
use aster_time::tsc::read_instant;
use core::hint::spin_loop;

use crate::alloc::string::ToString;
use super::MOUSE_CALLBACKS;
// use crate::event_type_codes::*;
use aster_input::event_type_codes::{EventType, RelAxis, MouseKeyEvent};

/// Data register (R/W)
static DATA_PORT: Once<IoPort<u8, ReadWriteAccess>> = Once::new();

/// Status register (R)
static STATUS_PORT: Once<IoPort<u8, ReadWriteAccess>> = Once::new();

/// IrqLine for i8042 mouse.
static IRQ_LINE: Once<SpinLock<IrqLine>> = Once::new();

// Controller commands
const DISABLE_MOUSE: u8 = 0xA7;
const ENABLE_MOUSE: u8 = 0xA8;
const DISABLE_KEYBOARD: u8 = 0xAD;
const ENABLE_KEYBOARD: u8 = 0xAE;
const MOUSE_WRITE: u8 = 0xD4;
const READ_CONFIG: u8 = 0x20;
const WRITE_CONFIG: u8 = 0x60;

// Mouse commands
const MOUSE_ENABLE: u8 = 0xF4;
const MOUSE_RESET: u8 = 0xFF;
const MOUSE_DEFAULT: u8 = 0xF6;

// Configure bits
const ENABLE_KEYBOARD_BIT: u8 = 0x1;
const ENABLE_MOUSE_BIT: u8 = 0x2;
const ENABLE_MOUSE_CLOCK_BIT: u8 = 0x20;

pub fn init() {
    log::error!("This is init in kernel/comps/mouse/src/i8042_mouse.rs");

    DATA_PORT.call_once(|| IoPort::acquire(0x60).unwrap());
    STATUS_PORT.call_once(|| IoPort::acquire(0x64).unwrap());

    init_i8042_controller();

    IRQ_LINE.call_once(|| {
        let mut irq_line = IrqLine::alloc().unwrap();
        irq_line.on_active(handle_mouse_input);

        let mut io_apic = IO_APIC.get().unwrap()[0].lock();
        io_apic.enable(12, irq_line.clone()).unwrap();

        SpinLock::new(irq_line)
    });

    init_mouse_device();

    aster_input::register_device("i8042_mouse".to_string(), Arc::new(I8042Mouse));
}


/// Initialize i8042 controller
fn init_i8042_controller() {
    // Disable keyborad and mouse
    STATUS_PORT.get().unwrap().write(DISABLE_MOUSE);
    STATUS_PORT.get().unwrap().write(DISABLE_KEYBOARD);

    // Clear the input buffer
    while DATA_PORT.get().unwrap().read() & 0x1 != 0 {
        let _ = DATA_PORT.get().unwrap().read();
    }

    // Set up the configuration
    STATUS_PORT.get().unwrap().write(READ_CONFIG); 
    let mut config = DATA_PORT.get().unwrap().read();
    config |= ENABLE_KEYBOARD_BIT; 
    config |= ENABLE_MOUSE_BIT; 
    config &= !ENABLE_MOUSE_CLOCK_BIT;

    STATUS_PORT.get().unwrap().write(WRITE_CONFIG);
    DATA_PORT.get().unwrap().write(config);

    // Enable keyboard and mouse
    STATUS_PORT.get().unwrap().write(ENABLE_KEYBOARD);
    STATUS_PORT.get().unwrap().write(ENABLE_MOUSE);
}

/// Initialize i8042 mouse
fn init_mouse_device() {
    // Send reset command
    STATUS_PORT.get().unwrap().write(MOUSE_WRITE);
    DATA_PORT.get().unwrap().write(MOUSE_RESET);
    wait_ack();

    // Set up default configuration
    STATUS_PORT.get().unwrap().write(MOUSE_WRITE);
    DATA_PORT.get().unwrap().write(MOUSE_DEFAULT);
    wait_ack();

    // Enable data reporting
    STATUS_PORT.get().unwrap().write(MOUSE_WRITE);
    DATA_PORT.get().unwrap().write(MOUSE_ENABLE);
    wait_ack();
}

/// Wait for controller's acknowledgement
fn wait_ack() {
    loop {
        if STATUS_PORT.get().unwrap().read() & 0x1 != 0 {
            let data = DATA_PORT.get().unwrap().read();
            if data == 0xFA {
                return 
            }
        }
        spin_loop();
    }
}

struct I8042Mouse;

impl InputDevice for I8042Mouse {
    fn metadata(&self) -> InputDeviceMeta {
        InputDeviceMeta {
            name: "i8042_mouse".to_string(),
            vendor_id: 0x2345,    // Replace with the actual vendor ID
            product_id: 0x6789,  // Replace with the actual product ID
            version: 2,          // Replace with the actual version
        }
    }
}

pub struct MouseState {
    buffer: [u8; 3],
    index: usize,
}

static MOUSE_STATE: Mutex<MouseState> = Mutex::new(MouseState { buffer: [0; 3], index: 0 });

fn handle_mouse_input(_trap_frame: &TrapFrame) {
    let byte = MousePacket::read_one_byte();

    let mut state = MOUSE_STATE.lock();

    if state.index == 0 && (byte & 0x08 == 0) {
        log::error!("Invalid first byte! Abort.");
        return;
    }
    let index = state.index;
    state.buffer[index] = byte;
    state.index += 1;

    if state.index == 3 {
        let packet = parse_input_packet(state.buffer);
        state.index = 0;
        handle_mouse_packet(packet);
    }
}

fn handle_mouse_packet(packet: MousePacket) {
    let event = parse_input_event(packet);  
    
    input_event(event, "i8042_mouse");

    // Fixme: the callbacks are going to be replaced.
    for callback in MOUSE_CALLBACKS.lock().iter() {
        callback();
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MousePacket {
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
    pub x_movement: i8,
    pub y_movement: i8,
    pub x_overflow: bool,
    pub y_overflow: bool,
}

impl MousePacket {
    fn read_one_byte() -> u8 {
        DATA_PORT.get().unwrap().read()
    }
}

#[derive(Debug, Clone, Copy)]
struct Status(u8);

impl Status {
    const STAT_OUTPUT_BUFFER_FULL: u8 = 0x01; /* Mouse output buffer full */

    fn read() -> Self {
        Self(STATUS_PORT.get().unwrap().read())
    }

    fn is_valid(&self) -> bool {
        self.0 != 0xFF
    }

    fn output_buffer_is_full(&self) -> bool {
        self.0 & Self::STAT_OUTPUT_BUFFER_FULL == 1
    }
}

fn parse_input_packet(packet: [u8; 3]) -> MousePacket {
    // let status = Status::read();
    // if !status.is_valid() {
    //     log::error!("invalid mouse input!");
    // }
    // if !status.output_buffer_is_full() {
    //     log::error!("No input.");
    // }

    // let byte0 = MousePacket::read_one_byte();
    // let byte1 = MousePacket::read_one_byte();
    // let byte2 = MousePacket::read_one_byte();

    log::error!("This is parse_input_packet in kernel/comps/mouse/src/i8042_mouse.rs");

    let byte0 = packet[0];
    let byte1 = packet[1];
    let byte2 = packet[2];

    MousePacket {
        left_button:   byte0 & 0x01 != 0,
        right_button:  byte0 & 0x02 != 0,
        middle_button: byte0 & 0x04 != 0,
        x_overflow:    byte0 & 0x40 != 0,
        y_overflow:    byte0 & 0x80 != 0,
        x_movement:    byte1 as i8,
        y_movement:   -(byte2 as i8),
    }
}

fn parse_input_event(packet: MousePacket) -> InputEvent {
    log::error!("The packet is: L={}, R={}, M={}, X={}, Y={}", packet.left_button, packet.right_button, packet.middle_button, packet.x_movement, packet.y_movement);

    // Get the current time in microseconds
    let now = read_instant();
    let time_in_microseconds = now.secs() * 1_000_000 + (now.nanos() / 1_000) as u64;

    if packet.x_movement != 0 {
        InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvRel as u16,
            code: RelAxis::RelX as u16,
            value: packet.x_movement as i32,
        }
    } else if packet.y_movement != 0 {
        InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvRel as u16,
            code: RelAxis::RelY as u16,
            value: packet.y_movement as i32,
        }
    } else if packet.left_button {
        InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvKey as u16,
            code: MouseKeyEvent::MouseLeft as u16,
            value: 1,
        }
    } else if packet.right_button {
        InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvKey as u16,
            code: MouseKeyEvent::MouseRight as u16,
            value: 1,
        }
    } else if packet.middle_button {
        InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvKey as u16,
            code: MouseKeyEvent::MouseMiddle as u16,
            value: 1,
        }
    } else {
        // Null input
        // log::error!("Wrong input for mouse!");
        InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvRel as u16,
            code: RelAxis::RelX as u16,
            value: 0,
        }
    }
}