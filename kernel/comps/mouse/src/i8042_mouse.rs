// SPDX-License-Identifier: MPL-2.0

//! The i8042 mouse driver.

use core::sync::atomic::{AtomicBool, Ordering};
use ostd::{
    arch::{device::io_port::ReadOnlyAccess, IO_APIC},
    io::IoPort,
    sync::SpinLock,
    trap::{IrqLine, TrapFrame},
};
use spin::Once;
use alloc::sync::Arc;
use aster_input::{InputDevice, InputDeviceMeta, InputEvent, input_event};
use aster_time::tsc::read_instant;

use crate::alloc::string::ToString;
use super::MOUSE_CALLBACKS;
use crate::event_type_codes::*;

/// Data register (R/W)
static DATA_PORT: Once<IoPort<u8, ReadOnlyAccess>> = Once::new();

/// Status register (R)
static STATUS_PORT: Once<IoPort<u8, ReadOnlyAccess>> = Once::new();

/// IrqLine for i8042 mouse.
static IRQ_LINE: Once<SpinLock<IrqLine>> = Once::new();

pub fn init() {
    log::error!("This is init in kernel/comps/mouse/src/i8042_mouse.rs");

    DATA_PORT.call_once(|| IoPort::acquire(0x60).unwrap());
    STATUS_PORT.call_once(|| IoPort::acquire(0x64).unwrap());
    IRQ_LINE.call_once(|| {
        let mut irq_line = IrqLine::alloc().unwrap();
        irq_line.on_active(handle_mouse_input);

        let mut io_apic = IO_APIC.get().unwrap()[0].lock();
        io_apic.enable(12, irq_line.clone()).unwrap();

        SpinLock::new(irq_line)
    });

    aster_input::register_device("i8042_mouse".to_string(), Arc::new(I8042Mouse));
}
struct I8042Mouse;

impl InputDevice for I8042Mouse {
    fn metadata(&self) -> InputDeviceMeta {
        InputDeviceMeta {
            name: "i8042_mouse".to_string(),
            vendor_id: 0x1234,    // Replace with the actual vendor ID
            product_id: 0x5678,  // Replace with the actual product ID
            version: 1,          // Replace with the actual version
        }
    }
}

fn handle_mouse_input(_trap_frame: &TrapFrame) {
    log::error!("This is handle_mouse_input in kernel/comps/mouse/src/i8042_mouse.rs");
    // let key = parse_inputkey();
    let packet = parse_input_packet();

    // TODO: mouse input events
    // Dispatch the input event
    let event = parse_input_event(packet);
    input_event(event);
    // input_event(InputEvent {
    //     time: time_in_microseconds, // Assign the current timestamp
    //     type_: 1,                   // EV_KEY (example type for key events)
    //     code: 0 as u16,           // Convert InputKey to a u16 representation
    //     value: 1,                   // Example value (1 for key press, 0 for release)
    // });

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

fn parse_input_packet() -> MousePacket {
    let status = Status::read();
    if !status.is_valid() {
        log::error!("invalid mouse input!");
    }
    if !status.output_buffer_is_full() {
        log::error!("No input.");
    }

    let byte0 = MousePacket::read_one_byte();
    let byte1 = MousePacket::read_one_byte();
    let byte2 = MousePacket::read_one_byte();

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
    // Get the current time in microseconds
    let now = read_instant();
    let time_in_microseconds = now.secs() * 1_000_000 + (now.nanos() / 1_000) as u64;

    if packet.x_movement != 0 {
        InputEvent {
            time: time_in_microseconds,
            type_: EV_REL,
            code: REL_X,
            value: packet.x_movement as i32,
        }
    } else if packet.y_movement != 0 {
        InputEvent {
            time: time_in_microseconds,
            type_: EV_REL,
            code: REL_Y,
            value: packet.y_movement as i32,
        }
    } else if packet.left_button {
        InputEvent {
            time: time_in_microseconds,
            type_: EV_KEY,
            code: BTN_LEFT,
            value: 1,
        }
    } else if packet.right_button {
        InputEvent {
            time: time_in_microseconds,
            type_: EV_KEY,
            code: BTN_RIGHT,
            value: 1,
        }
    } else if packet.middle_button {
        InputEvent {
            time: time_in_microseconds,
            type_: EV_KEY,
            code: BTN_MIDDLE,
            value: 1,
        }
    } else {
        // Unknown input
        log::error!("Wrong input for mouse!");
        InputEvent {
            time: time_in_microseconds,
            type_: EV_REL,
            code: REL_X,
            value: 0,
        }
    }
}