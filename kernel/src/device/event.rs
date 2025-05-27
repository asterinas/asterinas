#![expect(unused_variables)]

use super::*;
use crate::{
    events::IoEvents,
    fs::inode_handle::FileIo,
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    syscall::ClockId,
    util::MultiWrite,
};
use aster_input::{register_handler, InputHandler, InputEvent, event_type_codes::EventType};
use alloc::collections::VecDeque;
use spin::{Mutex, Once};
use aster_input::InputDevice;
use crate::fs::utils::IoctlCmd;
use crate::current_userspace;
use aster_input::event_type_codes::*;

const BITS_PER_WORD: usize = usize::BITS as usize;
const EV_BITMAP_LEN: usize = (EV_COUNT + BITS_PER_WORD - 1) / BITS_PER_WORD;
const KEY_BITMAP_LEN: usize = (KEY_COUNT + BITS_PER_WORD - 1) / BITS_PER_WORD;
const REL_BITMAP_LEN: usize = (REL_COUNT + BITS_PER_WORD - 1) / BITS_PER_WORD;
const ABS_BITMAP_LEN: usize = (ABS_COUNT + BITS_PER_WORD - 1) / BITS_PER_WORD;
const MSC_BITMAP_LEN: usize = (MSC_COUNT + BITS_PER_WORD - 1) / BITS_PER_WORD;
const LED_BITMAP_LEN: usize = (LED_COUNT + BITS_PER_WORD - 1) / BITS_PER_WORD;
const SND_BITMAP_LEN: usize = (SND_COUNT + BITS_PER_WORD - 1) / BITS_PER_WORD;
const FF_BITMAP_LEN: usize = (FF_COUNT + BITS_PER_WORD - 1) / BITS_PER_WORD;
const SW_BITMAP_LEN: usize = (SW_COUNT + BITS_PER_WORD - 1) / BITS_PER_WORD;
const PROP_BITMAP_LEN: usize = (PROP_COUNT + BITS_PER_WORD - 1) / BITS_PER_WORD;

const EVIOCGBIT_NR_MAX: u8 = EVIOCGBIT_NR + EV_COUNT as u8;

// use crate::syscall::ClockId::CLOCK_MONOTONIC;
use crate::syscall::clock_gettime::read_clock_input;

const NR_SHIFT: usize = 0;
const TYPE_SHIFT: usize = 8;
const SIZE_SHIFT: usize = 16;

const EVIOCGBIT_NR: u8 = (IoctlCmd::EVIOCGBIT as u32 & 0xFF) as u8;
const EVIOCGID_NR: u8 = (IoctlCmd::EVIOCGID as u32 & 0xFF) as u8;
const EVIOCGKEY_NR: u8 = (IoctlCmd::EVIOCGKEY as u32 & 0xFF) as u8;
const EVIOCGLED_NR: u8 = (IoctlCmd::EVIOCGLED as u32 & 0xFF) as u8;
const EVIOCGNAME_NR: u8 = (IoctlCmd::EVIOCGNAME as u32 & 0xFF) as u8;
const EVIOCGPHYS_NR: u8 = (IoctlCmd::EVIOCGPHYS as u32 & 0xFF) as u8;
const EVIOCGUNIQ_NR: u8 = (IoctlCmd::EVIOCGUNIQ as u32 & 0xFF) as u8;
const EVIOCGPROP_NR: u8 = (IoctlCmd::EVIOCGPROP as u32 & 0xFF) as u8;
const EVIOCGREP_NR: u8 = (IoctlCmd::EVIOCGREP as u32 & 0xFF) as u8;
const EVIOCGSW_NR: u8 = (IoctlCmd::EVIOCGSW as u32 & 0xFF) as u8;
const EVIOCGVERSION_NR: u8 = (IoctlCmd::EVIOCGVERSION as u32 & 0xFF) as u8;
const EVIOCSCLOCKID_NR: u8 = (IoctlCmd::EVIOCSCLOCKID as u32 & 0xFF) as u8;

// const EVIOCGBIT_NR: u8 = 0x20;
// const EVIOCGID_NR: u8 = 0x02;
// const EVIOCGKEY_NR: u8 = 0x18;
// const EVIOCGLED_NR: u8 = 0x19;
// const EVIOCGNAME_NR: u8 = 0x06;
// const EVIOCGPHYS_NR: u8 = 0x07;
// const EVIOCGUNIQ_NR: u8 = 0x08;
// const EVIOCGPROP_NR: u8 = 0x09;
// const EVIOCGREP_NR: u8 = 0x03;
// const EVIOCGSW_NR: u8 = 0x1b;
// const EVIOCGVERSION_NR: u8 = 0x01;
// const EVIOCSCLOCKID_NR: u8 = 0xa0;


#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEventLinux {
    pub sec: u64,    // Seconds (time.tv_sec or __sec)
    pub usec: u64,   // Microseconds (time.tv_usec or __usec)
    pub type_: u16,  // Event type
    pub code: u16,   // Event code
    pub value: i32,  // Event value
}

impl InputEventLinux {
    pub fn to_bytes(&self) -> [u8; 24] {
        let mut bytes = [0u8; 24];
        bytes[..8].copy_from_slice(&self.sec.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.usec.to_le_bytes());
        bytes[16..18].copy_from_slice(&self.type_.to_le_bytes());
        bytes[18..20].copy_from_slice(&self.code.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.value.to_le_bytes());
        bytes
    }
}

pub struct EventDevice {
    id: usize,
    clock_id: ClockId,
    event_queue: Arc<Mutex<VecDeque<InputEventLinux>>>,
    input_device: Arc<dyn InputDevice>,
    pollee: Pollee,
}

static EVENT_DEVICE_HANDLER: Once<Arc<EventDeviceHandler>> = Once::new();

impl EventDevice {
    pub fn new(id: usize, input_device: Arc<dyn InputDevice>) -> Arc<Self> {
        let clock_id = ClockId::CLOCK_MONOTONIC;
        let event_device = Arc::new(Self {
            id,
            clock_id,
            event_queue: Arc::new(Mutex::new(VecDeque::new())),
            input_device: input_device.clone(),
            pollee: Pollee::new(),
        });

        let metadata = input_device.metadata();
        println!(
            "InputDevice Metadata: name = {}",
            metadata.name
        );

        // Initialize the static handler if it hasn't been initialized yet
        let handler = EVENT_DEVICE_HANDLER.call_once(|| {
            Arc::new(EventDeviceHandler {
                event_devices: Mutex::new(Vec::new()), // Initialize the Mutex
            })
        });

        // Update the handler's weak reference to point to the new EventDevice
        handler.event_devices.lock().push(Arc::downgrade(&event_device));

        // Register the handler
        register_handler(handler.clone());

        // Connect the input_device to the handler
        aster_input::acquire_connection(input_device, handler.clone());

        event_device
    }

    pub fn push_event(&self, event: InputEventLinux) {
        let mut queue = self.event_queue.lock();
        if queue.len() >= 128 {
            queue.pop_front();
        }
        queue.push_back(event);
        // println!("Pushed event: {:?}", event);
        if event.type_ == EventType::EvSyn as u16 {
            // println!("EventDevice::push_event: SYN event detected");
            self.pollee.notify(IoEvents::IN);
        }
    }

    pub fn input_device(&self) -> Arc<dyn InputDevice> {
        Arc::clone(&self.input_device)
    }
}

impl Clone for EventDevice {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            clock_id: self.clock_id,
            event_queue: Arc::clone(&self.event_queue),
            input_device: Arc::clone(&self.input_device),
            pollee: self.pollee.clone(),
        }
    }
}

impl Device for EventDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(13, self.id as u32)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(Arc::new(self.clone())))
    }
}

impl Pollable for EventDevice {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        // println!("EventDevice::poll called with mask: {:?}", mask);

        // Use the Pollee mechanism to manage readiness and notifications
        self.pollee.poll_with(mask, poller, || {
            // Check if there are events in the queue
            let queue = self.event_queue.lock();
            if !queue.is_empty() {
                IoEvents::IN // Data is available to read
            } else {
                IoEvents::empty() // No events available
            }
        })
    }
}

impl FileIo for EventDevice {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        // println!("EventDevice::read called");
        let mut queue = self.event_queue.lock(); // Lock the event queue for thread-safe access
        if let Some(event) = queue.pop_front() { // Retrieve the oldest event from the queue
            let event_bytes = event.to_bytes(); // Serialize the event into bytes
            let mut reader = VmReader::from(&event_bytes[..]); // Create a reader for the serialized bytes
            writer.write(&mut reader)?; // Write the serialized event to the writer
            if queue.is_empty() {
                self.pollee.invalidate();
            }
            Ok(event_bytes.len()) // Return the size of the serialized event
        } else {
            Ok(0) // Return 0 if the queue is empty
        }
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        Ok(reader.remain())
    }

    fn ioctl(&self, cmd: crate::fs::utils::IoctlCmd, arg: usize) -> Result<i32> {
        // bits 0..7    : NR    (command number)
        // bits 8..15   : TYPE  (subsystem, like 'E' for evdev)
        // bits 16..29  : SIZE  (data size in bytes)
        // bits 30..31  : DIR   (_IOC_READ / _IOC_WRITE / etc.)

        // log::error!("-------------Coming into evdev-ioctl! cmd is {:#x}, arg is {:?}", cmd as u32, arg);

        let cmd_val = cmd as u32;
        // let cmd_val = cmd.as_u32();
        let cmd_nr  = ((cmd_val >> NR_SHIFT) & 0xFF) as u8;
        let cmd_type = ((cmd_val >> TYPE_SHIFT) & 0xFF) as u8;
        let cmd_size  = ((cmd_val >> SIZE_SHIFT) & 0x3FFF) as u16;

        match cmd_nr {
            EVIOCGBIT_NR..=EVIOCGBIT_NR_MAX => {
                // Example: rc = ioctl(fd, EVIOCGBIT(EV_REL, sizeof(dev->rel_bits)), dev->rel_bits);
                // Return value: error number
                // Parameter: 
                // Function: Fill `dev->xxx_bits` with kernel's `dev->xxx_bits`
                // Our implementation: Fill device's corresponding bitmap. To do this, we need to add these bitmaps
                // for each device when initialzied in kernel, indicating its supportive events. (true)
                log::error!("The cmd_type is {}", cmd_type);
                log::error!("The cmd_nr is {}", cmd_nr);
                let type_ = EventType::try_from(cmd_nr - 0x20).unwrap();
                let size = cmd_size;
                handle_eviocgbit(self.input_device(), type_, size, arg)
            }
            EVIOCGID_NR => {
                // Example: rc = ioctl(fd, EVIOCGID, &dev->ids);
                // Return value: error number
                // Parameter: 
                // Function: Fill `&dev->ids` with device's input_id struct, including bustype, vendor, product, version
                // Our implementation: Fill device's input_id struct (true)

                let id = self.input_device().metadata().id;
                current_userspace!().write_val(arg, &id)?;
                Ok(0)
            }
            EVIOCGKEY_NR => {
                // Example: rc = ioctl(fd, EVIOCGKEY(sizeof(dev->key_values)), dev->key_values);
                // Return value: error number
                // Parameter: 
                // Function: Fill `dev->key_values` with kernel's dev->key, which indicates key's current state
                // Our implementation: Call event_handle_get_val() with EV_KEY, copying kernel's dev->key to 
                // library's dev->key (true)

                // event_handle_get_val()
                let buf: [usize; KEY_BITMAP_LEN] = [0; KEY_BITMAP_LEN];
                current_userspace!().write_val(arg, &buf)?;

                Ok(0)
            }
            EVIOCGLED_NR => {
                // Example: rc = ioctl(fd, EVIOCGLED(sizeof(dev->led_values)), dev->led_values);
                // Return value: error number
                // Parameter: 
                // Function: Fill `dev->led_values` with kernel's dev->led, which indicates led's current state
                // Our implementation: Call event_handle_get_val() with EV_LED, copying kernel's dev->led to 
                // library's dev->led (true)

                // event_handle_get_val()
                let buf: [usize; LED_BITMAP_LEN] = [0; LED_BITMAP_LEN];
                current_userspace!().write_val(arg, &buf)?;
                Ok(0)
            }
            EVIOCGNAME_NR => {
                // Example: rc = ioctl(fd, EVIOCGNAME(sizeof(buf) - 1), buf);
                // Return value: error number
                // Parameter: 
                // Function: Fill `buf` with device's name, like "VirtualPS/2 VMware VMMouse\0"
                // Our implementation: Fill device's name (true)
                let mut buf = [0u8; 32];
                let name = self.input_device().metadata().name;
                buf[..name.len()].copy_from_slice(name.as_bytes());
                current_userspace!().write_val(arg, &buf)?;

                Ok(0)
            }
            EVIOCGPHYS_NR => {
                // Example: rc = ioctl(fd, EVIOCGPHYS(sizeof(buf) - 1), buf);
                // Return value: error number
                // Parameter: 
                // Function: Fill `buf` with device's physical location, like "isa0060/serio1/input1\0"
                // Our implementation: Fill any string (false)

                let mut buf = [0u8; 32];
                let phys = self.input_device().metadata().phys;
                buf[..phys.len()].copy_from_slice(phys.as_bytes());
                current_userspace!().write_val(arg, &buf)?;
                Ok(0)
            }
            EVIOCGUNIQ_NR => {
                // Example: rc = ioctl(fd, EVIOCGUNIQ(sizeof(buf) - 1), buf);
                // Return value: error number
                // Parameter: 
                // Function: Fill `buf` with device's unique number
                // Our implementation: Just Fill ENOENT (false)
                
                let mut buf = [0u8; 32];
                let uniq = self.input_device().metadata().uniq;
                buf[..uniq.len()].copy_from_slice(uniq.as_bytes());
                current_userspace!().write_val(arg, &buf)?;
                Ok(0)
            }
            EVIOCGPROP_NR => {
                // Example: rc = ioctl(fd, EVIOCGPROP(sizeof(dev->props)), dev->props);
                // Return value: error number
                // Parameter: 
                // Function: Fill `dev->props` with device's properties and quirks
                // Our implementation: Fill nothing, because our APIs do not use dev->props (false)

                let prop_bits = self.input_device().get_prop_bit();
                let mut bitmap: [usize; PROP_BITMAP_LEN] = [0; PROP_BITMAP_LEN];

                for prop in prop_bits {
                    let bit_index = prop as usize;
                    let word_index = bit_index / BITS_PER_WORD;
                    let bit_offset = bit_index % BITS_PER_WORD;
                
                    bitmap[word_index] |= 1 << bit_offset;
                }
                current_userspace!().write_val(arg, &bitmap)?;

                Ok(0)
            }
            EVIOCGREP_NR => {
                // Example: rc = ioctl(fd, EVIOCGREP, dev->rep_values);
                // Return value: error number
                // Parameter: 
                // Function: Fill `dev->rep_values` with kernel's dev->rep[0] and dev->rep[1]
                // Our implementation: Just claim our device does not support this function (false)

                println!("Impossible EVIOCGREP ioctl!");
                return_errno!(Errno::EINVAL); // Invalid argument error
            }
            EVIOCGSW_NR => {
                // Example: rc = ioctl(fd, EVIOCGSW(sizeof(dev->sw_values)), dev->sw_values);
                // Return value: error number
                // Parameter: 
                // Function: Fill `dev->sw_values` with kernel's dev->sw, which indicates sw's current state
                // Our implementation: Call event_handle_get_val() with EV_SW, copying kernel's dev->sw to 
                // library's dev->sw (true)

                let buf: [usize; SW_BITMAP_LEN] = [0; SW_BITMAP_LEN];
                current_userspace!().write_val(arg, &buf)?;
                Ok(0)
            }
            EVIOCGVERSION_NR => {
                // Example: rc = ioctl(fd, EVIOCGVERSION, &dev->driver_version);
                // Return value: error number
                // Parameter: 
                // Function: Fill `&dev->driver_version` with device's version
                // Our implementation: Fill device's version (true)
                let version = self.input_device().metadata().version;
                current_userspace!().write_val(arg, &version)?;
                Ok(0)
            }
            EVIOCSCLOCKID_NR => {
                // Example: rc = ioctl(dev->fd, EVIOCSCLOCKID, &clockid)
                // Return value: error number
                // Parameter: 
                // Function: Set the clock of the events
                // Our implementation: Set the clock of the events. In fact, our application always set CLOCK_MONOTOLIC (true)
                
                // let p :usize = current_userspace!().read_val(arg).unwrap();
                // let clock_id = p as ClockId;
                let clock_id = ClockId::CLOCK_MONOTONIC;
                // match clock_id {
                //     ClockId::CLOCK_REALTIME => {
                //         self.clock_id = ClockId::CLOCK_REALTIME;
                //     }
                //     ClockId::CLOCK_MONOTONIC => {
                //         self.clock_id = ClockId::CLOCK_MONOTONIC;
                //     }
                //     ClockId::CLOCK_BOOTTIME => {
                //         self.clock_id = ClockId::CLOCK_BOOTTIME;
                //     }
                //     _ => {
                //         println!("Unsupported clock_id -> {:?}", clock_id);
                //         return_errno!(Errno::EINVAL);
                //     }
                // }
                Ok(0)
            }
            _ => {
                println!("Event ioctl: Unsupported command -> {:#x}", cmd as u32);
                // println!("Event ioctl: Unsupported command -> {:#x}", cmd.as_u32());
                return_errno!(Errno::EINVAL); // Invalid argument error
            }
        }
    }
}

#[derive(Debug)]
pub struct EventDeviceHandler {
    event_devices: Mutex<Vec<Weak<EventDevice>>>, // Wrap in a Mutex for mutable access
}

impl InputHandler for EventDeviceHandler {
    /// Specifies the event types this handler can process.
    fn supported_event_types(&self) -> Vec<u16> {
        vec![EventType::EvSyn as u16, EventType::EvKey as u16, EventType::EvRel as u16] // Supports keyboard and mouse events
    }

    /// Handles the input event by pushing it to the event queue.
    fn handle_event(&self, event: InputEvent, str: &str) -> core::result::Result<(), core::convert::Infallible> {
        let devices = self.event_devices.lock();
        for weak_dev in devices.iter() {
            if let Some(event_device) = weak_dev.upgrade() {
                let metadata = event_device.input_device.metadata();
                let name = metadata.name.as_str();
                if name != str {
                    continue;
                }

                let time = read_clock_input(event_device.clock_id as i32).unwrap();

                // Convert InputEvent to InputEventLinux
                let linux_event = InputEventLinux {
                    sec: time.as_secs(),
                    usec: time.subsec_micros() as u64,
                    // sec: event.time / 1_000_000,
                    // usec: event.time % 1_000_000,
                    type_: event.type_,
                    code: event.code,
                    value: event.value,
                };

                event_device.push_event(linux_event);
            }
        }

        Ok(())
    }
}


// Implement the Pollable trait for Arc<EventDevice>
impl Pollable for Arc<EventDevice> {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.as_ref().poll(mask, poller)
    }
}

// Implement the FileIo trait for Arc<EventDevice>
impl FileIo for Arc<EventDevice> {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        println!("Arc<EventDevice>::read called");
        // Lock the event queue for thread-safe access
        let mut queue = self.event_queue.lock();
        
        // Retrieve the oldest event from the queue
        if let Some(event) = queue.pop_front() {
            // Serialize the event into bytes
            let event_bytes = event.to_bytes(); // Use the `to_bytes` method of `InputEventLinux`
            
            // Create a reader for the serialized bytes
            let mut reader = VmReader::from(&event_bytes[..]);
            
            // Write the serialized event to the writer
            writer.write(&mut reader)?;

            if queue.is_empty() {
                self.pollee.invalidate();
            }

            // Return the size of the serialized event
            Ok(event_bytes.len())
        } else {
            // Return 0 if the queue is empty
            Ok(0)
        }
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        self.as_ref().write(reader)
    }
}

fn handle_eviocgbit(dev: Arc<dyn InputDevice>, type_: EventType, _size: u16, arg: usize) -> Result<i32> {
    log::error!("-------------Coming into handle_eviocgbit!");
    match type_ {
        EventType::EvSyn => {
            let _ = handle_get_ev_bit(dev, arg);
        }
        EventType::EvKey => {
            let _ = handle_get_key_bit(dev, arg);
        }
        EventType::EvRel => {
            let _ = handle_get_rel_bit(dev, arg);
        }
        EventType::EvAbs => {
            let _ = handle_get_abs_bit(dev, arg);
        }
        EventType::EvMsc => {
            let _ = handle_get_msc_bit(dev, arg);
        }
        EventType::EvLed => {
            let _ = handle_get_led_bit(dev, arg);
        }
        EventType::EvSnd => {
            let _ = handle_get_snd_bit(dev, arg);
        }
        EventType::EvFf => {
            let _ = handle_get_ff_bit(dev, arg);
        }
        EventType::EvSw => {
            let _ = handle_get_sw_bit(dev, arg);
        }
        _ => {
            println!("handle_eviocgbit: Unsupportive bit type -> {:?}", type_);
            return_errno!(Errno::EINVAL); // Invalid argument error
        }
    }
    Ok(0)
}

fn handle_get_ev_bit(dev: Arc<dyn InputDevice>, arg: usize) -> Result<i32> {
    let bits = dev.get_ev_bit();
    let mut bitmap: [usize; EV_BITMAP_LEN] = [0; EV_BITMAP_LEN];

    for ev in bits {
        let bit_index = ev as usize;
        let word_index = bit_index / BITS_PER_WORD;
        let bit_offset = bit_index % BITS_PER_WORD;

        bitmap[word_index] |= 1 << bit_offset;
    }
    current_userspace!().write_val(arg, &bitmap)?;

    Ok(0)
}

fn handle_get_key_bit(dev: Arc<dyn InputDevice>, arg: usize) -> Result<i32> {
    let bits = dev.get_key_bit();
    let mut bitmap: [usize; KEY_BITMAP_LEN] = [0; KEY_BITMAP_LEN];

    for key in bits {
        let bit_index = key as usize;
        let word_index = bit_index / BITS_PER_WORD;
        let bit_offset = bit_index % BITS_PER_WORD;

        bitmap[word_index] |= 1 << bit_offset;
    }

    // Keyboard bitmap: 0x402000000_03803078F800D001_FEFFFFDFFFEFFFFF_FFFFFFFFFFFFFFFE
    if dev.metadata().name.contains("keyboard") {
        bitmap[0] = 0xFFFFFFFFFFFFFFFE;
        bitmap[1] = 0xFEFFFFDFFFEFFFFF;
        bitmap[2] = 0x3803078F800D001;
        bitmap[3] = 0x402000000;
    }

    current_userspace!().write_val(arg, &bitmap)?;

    Ok(0)
}

fn handle_get_rel_bit(dev: Arc<dyn InputDevice>, arg: usize) -> Result<i32> {
    let bits = dev.get_rel_bit();
    let mut bitmap: [usize; REL_BITMAP_LEN] = [0; REL_BITMAP_LEN];

    for rel in bits {
        let bit_index = rel as usize;
        let word_index = bit_index / BITS_PER_WORD;
        let bit_offset = bit_index % BITS_PER_WORD;

        bitmap[word_index] |= 1 << bit_offset;
    }
    current_userspace!().write_val(arg, &bitmap)?;

    Ok(0)
}

fn handle_get_msc_bit(dev: Arc<dyn InputDevice>, arg: usize) -> Result<i32> {
    let bits = dev.get_msc_bit();
    let mut bitmap: [usize; MSC_BITMAP_LEN] = [0; MSC_BITMAP_LEN];

    for msc in bits {
        let bit_index = msc as usize;
        let word_index = bit_index / BITS_PER_WORD;
        let bit_offset = bit_index % BITS_PER_WORD;

        bitmap[word_index] |= 1 << bit_offset;
    }
    current_userspace!().write_val(arg, &bitmap)?;

    Ok(0)
}

fn handle_get_led_bit(dev: Arc<dyn InputDevice>, arg: usize) -> Result<i32> {
    let bits = dev.get_led_bit();
    let mut bitmap: [usize; LED_BITMAP_LEN] = [0; LED_BITMAP_LEN];

    for led in bits {
        let bit_index = led as usize;
        let word_index = bit_index / BITS_PER_WORD;
        let bit_offset = bit_index % BITS_PER_WORD;

        bitmap[word_index] |= 1 << bit_offset;
    }
    current_userspace!().write_val(arg, &bitmap)?;

    Ok(0)
}

fn handle_get_abs_bit(dev: Arc<dyn InputDevice>, arg: usize) -> Result<i32> {
    let bitmap: [usize; ABS_BITMAP_LEN] = [0; ABS_BITMAP_LEN];
    current_userspace!().write_val(arg, &bitmap)?;

    Ok(0)
}

fn handle_get_snd_bit(dev: Arc<dyn InputDevice>, arg: usize) -> Result<i32> {
    let bitmap: [usize; SND_BITMAP_LEN] = [0; SND_BITMAP_LEN];
    current_userspace!().write_val(arg, &bitmap)?;

    Ok(0)
}

fn handle_get_ff_bit(dev: Arc<dyn InputDevice>, arg: usize) -> Result<i32> {
    let bitmap: [usize; FF_BITMAP_LEN] = [0; FF_BITMAP_LEN];
    current_userspace!().write_val(arg, &bitmap)?;

    Ok(0)
}

fn handle_get_sw_bit(dev: Arc<dyn InputDevice>, arg: usize) -> Result<i32> {
    let bitmap: [usize; SW_BITMAP_LEN] = [0; SW_BITMAP_LEN];
    current_userspace!().write_val(arg, &bitmap)?;

    Ok(0)
}

// Implement the Device trait for Arc<EventDevice>
impl Device for Arc<EventDevice> {
    fn type_(&self) -> DeviceType {
        self.as_ref().type_()
    }

    fn id(&self) -> DeviceId {
        self.as_ref().id()
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        self.as_ref().open()
    }
}
