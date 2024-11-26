// SPDX-License-Identifier: MPL-2.0

// Modified from input.rs in virtio-drivers project
//
// MIT License
//
// Copyright (c) 2022-2023 Ant Group
// Copyright (c) 2019-2020 rCore Developers
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
//

pub mod device;
use aster_util::safe_ptr::SafePtr;
use ostd::{io_mem::IoMem, Pod};

use crate::transport::VirtioTransport;

pub static DEVICE_NAME: &str = "Virtio-Input";

/// Select value used for [`device::InputDevice::query_config_select()`].
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum InputConfigSelect {
    /// Invalid configuration selection.
    Unset = 0x00,
    /// Returns the name of the device, subsel is zero.
    IdName = 0x01,
    /// Returns the serial number of the device, subsel is zero.
    IdSerial = 0x02,
    /// Returns ID information of the device, subsel is zero.
    IdDevids = 0x03,
    /// Returns input properties of the device, subsel is zero.
    /// Individual bits in the bitmap correspond to INPUT_PROP_* constants used
    /// by the underlying evdev implementation.
    PropBits = 0x10,
    /// subsel specifies the event type using EV_* constants in the underlying
    /// evdev implementation. If size is non-zero the event type is supported
    /// and a bitmap of supported event codes is returned. Individual
    /// bits in the bitmap correspond to implementation-defined input event codes,
    /// for example keys or pointing device axes.
    EvBits = 0x11,
    /// subsel specifies the absolute axis using ABS_* constants in the underlying
    /// evdev implementation. Information about the axis will be returned.
    AbsInfo = 0x12,
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct VirtioInputConfig {
    /// write only
    select: u8,
    /// write only
    subsel: u8,
    /// read only
    size: u8,
    _reversed: [u8; 5],
    /// read only
    data: [u8; 128],
}

impl VirtioInputConfig {
    pub(self) fn new(transport: &dyn VirtioTransport) -> SafePtr<Self, IoMem> {
        let memory = transport.device_config_mem().unwrap();
        SafePtr::new(memory, 0)
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
struct AbsInfo {
    min: u32,
    max: u32,
    fuzz: u32,
    flat: u32,
    res: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
struct DevIds {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

/// Both queues use the same `virtio_input_event` struct. `type`, `code` and `value`
/// are filled according to the Linux input layer (evdev) interface.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct VirtioInputEvent {
    /// Event type.
    pub event_type: u16,
    /// Event code.
    pub code: u16,
    /// Event value.
    pub value: u32,
}

const QUEUE_EVENT: u16 = 0;
const QUEUE_STATUS: u16 = 1;
