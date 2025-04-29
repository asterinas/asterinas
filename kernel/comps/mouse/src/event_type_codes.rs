// SPDX-License-Identifier: MPL-2.0

/// Event type constants
pub const EV_SYN: u16 = 0x00; 
pub const EV_KEY: u16 = 0x01; 
pub const EV_REL: u16 = 0x02; 
pub const EV_ABS: u16 = 0x03; 

/// Event code constants for EV_REL
pub const REL_X: u16 = 0x00; 
pub const REL_Y: u16 = 0x01; 
pub const REL_WHEEL: u16 = 0x08; 

/// Event code constants for EV_KEY
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;