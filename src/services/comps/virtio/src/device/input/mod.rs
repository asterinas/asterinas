pub mod device;
use jinux_pci::{capability::vendor::virtio::CapabilityVirtioData, util::BAR};
use jinux_util::frame_ptr::InFramePtr;

/// Select value used for [`VirtIOInput::query_config_select()`].
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum InputConfigSelect {
    /// Returns the name of the device, in u.string. subsel is zero.
    IdName = 0x01,
    /// Returns the serial number of the device, in u.string. subsel is zero.
    IdSerial = 0x02,
    /// Returns ID information of the device, in u.ids. subsel is zero.
    IdDevids = 0x03,
    /// Returns input properties of the device, in u.bitmap. subsel is zero.
    /// Individual bits in the bitmap correspond to INPUT_PROP_* constants used
    /// by the underlying evdev implementation.
    PropBits = 0x10,
    /// subsel specifies the event type using EV_* constants in the underlying
    /// evdev implementation. If size is non-zero the event type is supported
    /// and a bitmap of supported event codes is returned in u.bitmap. Individual
    /// bits in the bitmap correspond to implementation-defined input event codes,
    /// for example keys or pointing device axes.
    EvBits = 0x11,
    /// subsel specifies the absolute axis using ABS_* constants in the underlying
    /// evdev implementation. Information about the axis will be returned in u.abs.
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
    pub(crate) fn new(cap: &CapabilityVirtioData, bars: [Option<BAR>; 6]) -> InFramePtr<Self> {
        let bar = cap.bar;
        let offset = cap.offset;
        match bars[bar as usize].expect("Virtio pci block cfg:bar is none") {
            BAR::Memory(address, _, _, _) => InFramePtr::new(address as usize + offset as usize)
                .expect("can not get in frame ptr for virtio block config"),
            BAR::IO(_, _) => {
                panic!("Virtio pci block cfg:bar is IO type")
            }
        }
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
struct DevIDs {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

/// Both queues use the same `virtio_input_event` struct. `type`, `code` and `value`
/// are filled according to the Linux input layer (evdev) interface.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct InputEvent {
    /// Event type.
    pub event_type: u16,
    /// Event code.
    pub code: u16,
    /// Event value.
    pub value: u32,
}

const QUEUE_EVENT: usize = 0;
const QUEUE_STATUS: usize = 1;

// a parameter that can change
const QUEUE_SIZE: usize = 64;
