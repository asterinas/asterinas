// SPDX-License-Identifier: MPL-2.0

//! The i8042 mouse driver.

use core::sync::atomic::{AtomicBool, Ordering};
use ostd::{
    arch::{device::io_port::ReadWriteAccess, trap::TrapFrame},
    io::IoPort,
    sync::SpinLock,
    sync::Mutex,
    trap::irq::IrqLine,
};
use spin::Once;
use alloc::sync::Arc;
use alloc::vec::Vec;
use aster_input::{InputDevice, InputDeviceMeta, InputEvent, input_event};
use aster_time::tsc::read_instant;
use core::hint::spin_loop;

use crate::alloc::string::ToString;
use super::MOUSE_CALLBACKS;
use aster_input::event_type_codes::{EventType, RelAxis, MouseKeyEvent};
use crate::MOUSE_WRITE;



use crate::DATA_PORT;
use crate::STATUS_PORT;
use crate::MOUSE_IRQ_LINE;

pub fn init() {
    log::error!("This is init in kernel/comps/mouse/src/i8042_mouse.rs");

    aster_input::register_device("i8042_mouse".to_string(), Arc::new(I8042Mouse));
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

pub fn handle_mouse_input(_trap_frame: &TrapFrame) {
    // log::error!("-----This is handle_mouse_input in kernel/comps/i8042_controller/src/i8042_mouse.rs");
    let byte = MousePacket::read_one_byte();

    let mut state = MOUSE_STATE.lock();

    if state.index == 0 && (byte & 0x08 == 0) {
        log::error!("Invalid first byte! Abort.");
        state.index = 0;
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

use ostd::prelude::println;
fn handle_mouse_packet(packet: MousePacket) {
    // Parse multiple events from the packet
    let mut events = parse_input_events(packet);

    // Add a SYNC event to signal the end of the event group
    events.push(InputEvent {
        time: 0,
        type_: EventType::EvSyn as u16,
        code: 0, // SYN_REPORT
        value: 0,
    });
    // Process each event
    for event in events {
        println!("Event: {:?}", event);
        input_event(event, "i8042_mouse");
    }

    // FIXME: the callbacks are going to be replaced.
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

    log::error!("This is parse_input_packet in kernel/comps/mouse/src/i8042_mouse.rs packet: {:?}", packet);

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

fn parse_input_events(packet: MousePacket) -> Vec<InputEvent> {
    let mut events = Vec::new();

    // Get the current time in microseconds
    let now = read_instant();
    let time_in_microseconds = now.secs() * 1_000_000 + (now.nanos() / 1_000) as u64;

    // Add X movement event if applicable
    if packet.x_movement != 0 {
        events.push(InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvRel as u16,
            code: RelAxis::RelX as u16,
            value: packet.x_movement as i32,
        });
    }

    // Add Y movement event if applicable
    if packet.y_movement != 0 {
        events.push(InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvRel as u16,
            code: RelAxis::RelY as u16,
            value: packet.y_movement as i32,
        });
    }

    // Add button press/release events
    if packet.left_button {
        events.push(InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvKey as u16,
            code: MouseKeyEvent::MouseLeft as u16,
            value: 1,
        });
    }
    if packet.right_button {
        events.push(InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvKey as u16,
            code: MouseKeyEvent::MouseRight as u16,
            value: 1,
        });
    }
    if packet.middle_button {
        events.push(InputEvent {
            time: time_in_microseconds,
            type_: EventType::EvKey as u16,
            code: MouseKeyEvent::MouseMiddle as u16,
            value: 1,
        });
    }

    // Return the list of events
    events
}