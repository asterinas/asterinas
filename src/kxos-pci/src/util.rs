extern crate kxos_frame;
use crate::capability::Capability;
use alloc::vec::Vec;
use bitflags::bitflags;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CSpaceAccessMethod {
    // The legacy, deprecated (as of PCI 2.0) IO-range method.
    // Until/unless there is a relevant platform that requires this, leave it out.
    // IO_Mechanism_2
    /// The legacy (pre-PCIe) 2-IO port method as specified on page 50 of PCI Local Bus
    /// Specification 3.0.
    IO,
    // PCIe memory-mapped configuration space access
    //MemoryMapped(*mut u8),
}

// All IO-bus ops are 32-bit, we mask and shift to get the values we want.

impl CSpaceAccessMethod {
    pub fn read8(self, loc: Location, offset: u16) -> u8 {
        let val = self.read32(loc, offset & 0b11111100);
        ((val >> ((offset as usize & 0b11) << 3)) & 0xFF) as u8
    }

    /// Returns a value in native endian.
    pub fn read16(self, loc: Location, offset: u16) -> u16 {
        let val = self.read32(loc, offset & 0b11111100);
        ((val >> ((offset as usize & 0b10) << 3)) & 0xFFFF) as u16
    }

    /// Returns a value in native endian.
    pub fn read32(self, loc: Location, offset: u16) -> u32 {
        debug_assert!(
            (offset & 0b11) == 0,
            "misaligned PCI configuration dword u32 read"
        );
        match self {
            CSpaceAccessMethod::IO => {
                kxos_frame::device::pci::PCI_ADDRESS_PORT
                    .write_u32(loc.encode() | ((offset as u32) & 0b11111100));
                kxos_frame::device::pci::PCI_DATA_PORT.read_u32().to_le()
            } //MemoryMapped(ptr) => {
              //    // FIXME: Clarify whether the rules for GEP/GEPi forbid using regular .offset() here.
              //    ::core::intrinsics::volatile_load(::core::intrinsics::arith_offset(ptr, offset as usize))
              //}
        }
    }

    pub fn write8(self, loc: Location, offset: u16, val: u8) {
        let old = self.read32(loc, offset);
        let dest = offset as usize & 0b11 << 3;
        let mask = (0xFF << dest) as u32;
        self.write32(loc, offset, ((val as u32) << dest | (old & !mask)).to_le());
    }

    /// Converts val to little endian before writing.
    pub fn write16(self, loc: Location, offset: u16, val: u16) {
        let old = self.read32(loc, offset);
        let dest = offset as usize & 0b10 << 3;
        let mask = (0xFFFF << dest) as u32;
        self.write32(loc, offset, ((val as u32) << dest | (old & !mask)).to_le());
    }

    /// Takes a value in native endian, converts it to little-endian, and writes it to the PCI
    /// device configuration space at register `offset`.
    pub fn write32(self, loc: Location, offset: u16, val: u32) {
        debug_assert!(
            (offset & 0b11) == 0,
            "misaligned PCI configuration dword u32 read"
        );
        match self {
            CSpaceAccessMethod::IO => {
                kxos_frame::device::pci::PCI_ADDRESS_PORT
                    .write_u32(loc.encode() | (offset as u32 & 0b11111100));
                kxos_frame::device::pci::PCI_ADDRESS_PORT.write_u32(val.to_le())
            } //MemoryMapped(ptr) => {
              //    // FIXME: Clarify whether the rules for GEP/GEPi forbid using regular .offset() here.
              //    ::core::intrinsics::volatile_load(::core::intrinsics::arith_offset(ptr, offset as usize))
              //}
        }
    }
}

/// Physical location of a device on the bus
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl Location {
    #[inline(always)]
    fn encode(self) -> u32 {
        (1 << 31)
            | ((self.bus as u32) << 16)
            | (((self.device as u32) & 0b11111) << 11)
            | (((self.function as u32) & 0b111) << 8)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Identifier {
    pub vendor_id: u16,
    pub device_id: u16,
    pub revision_id: u8,
    pub prog_if: u8,
    pub class: u8,
    pub subclass: u8,
}

bitflags! {
    pub struct Command: u16 {
        const IO_SPACE                  = 0x0001;
        const MEMORY_SPACE              = 0x0002;
        const BUS_MASTER                = 0x0004;
        const SPECIAL_CYCLES            = 0x0008;
        const MWI_ENABLE                = 0x0010;
        const VGA_PALETTE_SNOOP         = 0x0020;
        const PARITY_ERROR_RESPONSE     = 0x0040;
        const STEPPING_CONTROL          = 0x0080;
        const SERR_ENABLE               = 0x0100;
        const FAST_BACK_TO_BACK_ENABLE  = 0x0200;
        const INTERRUPT_DISABLE         = 0x0400;
        const RESERVED_11               = 0x0800;
        const RESERVED_12               = 0x1000;
        const RESERVED_13               = 0x2000;
        const RESERVED_14               = 0x4000;
        const RESERVED_15               = 0x8000;
    }
}

bitflags! {
    pub struct Status: u16 {
        const RESERVED_0                = 0x0001;
        const RESERVED_1                = 0x0002;
        const RESERVED_2                = 0x0004;
        const INTERRUPT_STATUS          = 0x0008;
        const CAPABILITIES_LIST         = 0x0010;
        const MHZ66_CAPABLE             = 0x0020;
        const RESERVED_6                = 0x0040;
        const FAST_BACK_TO_BACK_CAPABLE = 0x0080;
        const MASTER_DATA_PARITY_ERROR  = 0x0100;
        const DEVSEL_MEDIUM_TIMING      = 0x0200;
        const DEVSEL_SLOW_TIMING        = 0x0400;
        const SIGNALED_TARGET_ABORT     = 0x0800;
        const RECEIVED_TARGET_ABORT     = 0x1000;
        const RECEIVED_MASTER_ABORT     = 0x2000;
        const SIGNALED_SYSTEM_ERROR     = 0x4000;
        const DETECTED_PARITY_ERROR     = 0x8000;
    }
}

bitflags! {
    pub struct BridgeControl: u16 {
        const PARITY_ERROR_RESPONSE_ENABLE = 0x0001;
        const SERR_ENABLE               = 0x0002;
        const ISA_ENABLE                = 0x0004;
        const VGA_ENABLE                = 0x0008;
        const RESERVED_4                = 0x0010;
        const MASTER_ABORT_MODE         = 0x0020;
        const SECONDARY_BUS_RESET       = 0x0040;
        const FAST_BACK_TO_BACK_ENABLE  = 0x0080;
        const PRIMARY_DISCARD_TIMER     = 0x0100;
        const SECONDARY_DISCARD_TIMER   = 0x0200;
        const DISCARD_TIMER_STATUS      = 0x0400;
        const DISCARD_TIMER_SERR_ENABLED = 0x0800;
        const RESERVED_12               = 0x1000;
        const RESERVED_13               = 0x2000;
        const RESERVED_14               = 0x4000;
        const RESERVED_15               = 0x8000;
    }
}

/// A device on the PCI bus.
///
/// Although accessing configuration space may be expensive, it is not cached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PCIDevice {
    pub loc: Location,
    pub id: Identifier,
    pub command: Command,
    pub status: Status,
    pub cache_line_size: u8,
    pub latency_timer: u8,
    pub multifunction: bool,
    pub bist_capable: bool,
    pub bars: [Option<BAR>; 6],
    pub kind: DeviceKind,
    pub pic_interrupt_line: u8,
    pub interrupt_pin: Option<InterruptPin>,
    pub cspace_access_method: CSpaceAccessMethod,
    pub capabilities: Vec<Capability>,
}

pub enum PCIScanError {}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Prefetchable {
    Yes,
    No,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Type {
    Bits32,
    Bits64,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DeviceKind {
    Device(DeviceDetails),
    PciBridge(PciBridgeDetails),
    CardbusBridge(CardbusBridgeDetails),
    Unknown,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DeviceDetails {
    pub cardbus_cis_ptr: u32,
    pub subsystem_vendor_id: u16,
    pub subsystem_id: u16,
    pub expansion_rom_base_addr: u32,
    pub min_grant: u8,
    pub max_latency: u8,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PciBridgeDetails {
    pub primary_bus: u8,
    pub secondary_bus: u8,
    pub subordinate_bus: u8,
    pub secondary_latency_timer: u8,
    pub io_base: u32,
    pub io_limit: u32,
    pub secondary_status: Status,
    pub mem_base: u32,
    pub mem_limit: u32,
    pub prefetchable_mem_base: u64,
    pub prefetchable_mem_limit: u64,
    pub expansion_rom_base_addr: u32,
    pub bridge_control: BridgeControl,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CardbusBridgeDetails {
    pub socket_base_addr: u32,
    pub secondary_status: Status,
    pub pci_bus: u8,
    pub cardbus_bus: u8,
    pub subordinate_bus: u8,
    pub cardbus_latency_timer: u8,
    pub mem_base_0: u32,
    pub mem_limit_0: u32,
    pub mem_base_1: u32,
    pub mem_limit_1: u32,
    pub io_base_0: u32,
    pub io_limit_0: u32,
    pub io_base_1: u32,
    pub io_limit_1: u32,
    pub subsystem_device_id: u16,
    pub subsystem_vendor_id: u16,
    pub legacy_mode_base_addr: u32,
    pub bridge_control: BridgeControl,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum InterruptPin {
    INTA = 1,
    INTB,
    INTC,
    INTD,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BAR {
    Memory(u64, u32, Prefetchable, Type),
    IO(u32, u32),
}

impl BAR {
    pub fn decode(loc: Location, am: CSpaceAccessMethod, idx: u16) -> (Option<BAR>, usize) {
        let raw = am.read32(loc, 16 + (idx << 2));
        am.write32(loc, 16 + (idx << 2), !0);
        let len_encoded = am.read32(loc, 16 + (idx << 2));
        am.write32(loc, 16 + (idx << 2), raw);
        if raw == 0 && len_encoded == 0 {
            return (None, idx as usize + 1);
        }
        if raw & 1 == 0 {
            let mut bits64 = false;
            let base: u64 = match (raw & 0b110) >> 1 {
                0 => (raw & !0xF) as u64,
                2 => {
                    bits64 = true;
                    ((raw & !0xF) as u64) | ((am.read32(loc, 16 + ((idx + 1) << 2)) as u64) << 32)
                }
                _ => {
                    debug_assert!(false, "bad type in memory BAR");
                    return (None, idx as usize + 1);
                }
            };
            let len = !(len_encoded & !0xF).wrapping_add(1);
            (
                Some(BAR::Memory(
                    base,
                    len,
                    if raw & 0b1000 == 0 {
                        Prefetchable::No
                    } else {
                        Prefetchable::Yes
                    },
                    if bits64 { Type::Bits64 } else { Type::Bits32 },
                )),
                if bits64 { idx + 2 } else { idx + 1 } as usize,
            )
        } else {
            let len = !(len_encoded & !0x3) + 1;
            (Some(BAR::IO(raw & !0x3, len)), idx as usize + 1)
        }
    }
}

pub(crate) struct BusScan {
    loc: Location,
    am: CSpaceAccessMethod,
}

impl BusScan {
    fn done(&self) -> bool {
        if self.loc.bus == 255 && self.loc.device == 31 && self.loc.function == 7 {
            true
        } else {
            false
        }
    }

    fn increment(&mut self) {
        // TODO: Decide whether this is actually nicer than taking a u16 and incrementing until it
        // wraps.
        if self.loc.function < 7 {
            self.loc.function += 1;
            return;
        } else {
            self.loc.function = 0;
            if self.loc.device < 31 {
                self.loc.device += 1;
                return;
            } else {
                self.loc.device = 0;
                if self.loc.bus == 255 {
                    self.loc.device = 31;
                    self.loc.device = 7;
                } else {
                    self.loc.bus += 1;
                    return;
                }
            }
        }
    }
}

impl ::core::iter::Iterator for BusScan {
    type Item = PCIDevice;
    #[inline]
    fn next(&mut self) -> Option<PCIDevice> {
        // FIXME: very naive atm, could be smarter and waste much less time by only scanning used
        // busses.
        let mut ret = None;
        loop {
            if self.done() {
                return ret;
            }
            ret = probe_function(self.loc, self.am);
            self.increment();
            if ret.is_some() {
                return ret;
            }
        }
    }
}

fn probe_function(loc: Location, am: CSpaceAccessMethod) -> Option<PCIDevice> {
    // FIXME: it'd be more efficient to use read32 and decode separately.
    let vid = am.read16(loc, 0);
    if vid == 0xFFFF {
        return None;
    }
    let did = am.read16(loc, 2);
    let command = Command::from_bits_truncate(am.read16(loc, 4));
    let status = Status::from_bits_truncate(am.read16(loc, 6));
    let rid = am.read8(loc, 8);
    let prog_if = am.read8(loc, 9);
    let subclass = am.read8(loc, 10);
    let class = am.read8(loc, 11);
    let id = Identifier {
        vendor_id: vid,
        device_id: did,
        revision_id: rid,
        prog_if: prog_if,
        class: class,
        subclass: subclass,
    };
    let cache_line_size = am.read8(loc, 12);
    let latency_timer = am.read8(loc, 13);
    let bist_capable = am.read8(loc, 15) & (1 << 7) != 0;
    let hdrty_mf = am.read8(loc, 14);
    let hdrty = hdrty_mf & !(1 << 7);
    let mf = hdrty_mf & (1 << 7) != 0;
    let pic_interrupt_line = am.read8(loc, 0x3C);
    let interrupt_pin = match am.read8(loc, 0x3D) {
        1 => Some(InterruptPin::INTA),
        2 => Some(InterruptPin::INTB),
        3 => Some(InterruptPin::INTC),
        4 => Some(InterruptPin::INTD),
        _ => None,
    };
    let kind;
    let max;

    match hdrty {
        0 => {
            max = 6;
            kind = DeviceKind::Device(DeviceDetails {
                cardbus_cis_ptr: am.read32(loc, 0x28),
                subsystem_vendor_id: am.read16(loc, 0x2C),
                subsystem_id: am.read16(loc, 0x2E),
                expansion_rom_base_addr: am.read32(loc, 0x30),
                min_grant: am.read8(loc, 0x3E),
                max_latency: am.read8(loc, 0x3F),
            });
        }
        1 => {
            max = 2;
            kind = DeviceKind::PciBridge(PciBridgeDetails {
                primary_bus: am.read8(loc, 0x18),
                secondary_bus: am.read8(loc, 0x19),
                subordinate_bus: am.read8(loc, 0x1a),
                secondary_latency_timer: am.read8(loc, 0x1b),
                secondary_status: Status::from_bits_truncate(am.read16(loc, 0x1e)),
                io_base: (am.read8(loc, 0x1c) as u32 & 0xF0) << 8
                    | (am.read16(loc, 0x30) as u32) << 16,
                io_limit: 0xFFF
                    | (am.read8(loc, 0x1d) as u32 & 0xF0) << 8
                    | (am.read16(loc, 0x32) as u32) << 16,
                mem_base: (am.read16(loc, 0x20) as u32 & 0xFFF0) << 16,
                mem_limit: 0xFFFFF | (am.read16(loc, 0x22) as u32 & 0xFFF0) << 16,
                prefetchable_mem_base: (am.read16(loc, 0x24) as u64 & 0xFFF0) << 16
                    | am.read32(loc, 0x28) as u64,
                prefetchable_mem_limit: 0xFFFFF
                    | (am.read16(loc, 0x26) as u64 & 0xFFF0) << 16
                    | am.read32(loc, 0x2c) as u64,
                expansion_rom_base_addr: am.read32(loc, 0x38),
                bridge_control: BridgeControl::from_bits_truncate(am.read16(loc, 0x3e)),
            });
        }
        2 => {
            max = 0;
            kind = DeviceKind::CardbusBridge(CardbusBridgeDetails {
                socket_base_addr: am.read32(loc, 0x10),
                secondary_status: Status::from_bits_truncate(am.read16(loc, 0x16)),
                pci_bus: am.read8(loc, 0x18),
                cardbus_bus: am.read8(loc, 0x19),
                subordinate_bus: am.read8(loc, 0x1a),
                cardbus_latency_timer: am.read8(loc, 0x1b),
                mem_base_0: am.read32(loc, 0x1c),
                mem_limit_0: am.read32(loc, 0x20),
                mem_base_1: am.read32(loc, 0x24),
                mem_limit_1: am.read32(loc, 0x28),
                io_base_0: am.read32(loc, 0x2c),
                io_limit_0: am.read32(loc, 0x30),
                io_base_1: am.read32(loc, 0x34),
                io_limit_1: am.read32(loc, 0x38),
                bridge_control: BridgeControl::from_bits_truncate(am.read16(loc, 0x3e)),
                subsystem_device_id: am.read16(loc, 0x40),
                subsystem_vendor_id: am.read16(loc, 0x42),
                legacy_mode_base_addr: am.read32(loc, 0x44),
            });
        }
        _ => {
            max = 0;
            kind = DeviceKind::Unknown;
            debug_assert!(
                false,
                "pci: unknown device header type {} for {:?} {:?}",
                hdrty, loc, id
            );
        }
    };

    let capabilities = if status.contains(Status::CAPABILITIES_LIST) {
        Capability::device_capabilities(loc)
    } else {
        Vec::new()
    };

    let mut bars = [None, None, None, None, None, None];
    let mut i = 0;
    while i < max {
        let (bar, next) = BAR::decode(loc, am, i as u16);
        bars[i] = bar;
        i = next;
    }

    Some(PCIDevice {
        loc: loc,
        id: id,
        command: command,
        status: status,
        cache_line_size: cache_line_size,
        latency_timer: latency_timer,
        multifunction: mf,
        bist_capable: bist_capable,
        bars: bars,
        kind: kind,
        pic_interrupt_line: pic_interrupt_line,
        interrupt_pin: interrupt_pin,
        cspace_access_method: am,
        capabilities: capabilities,
    })
}

pub(crate) fn scan_bus(am: CSpaceAccessMethod) -> BusScan {
    BusScan {
        loc: Location {
            bus: 0,
            device: 0,
            function: 0,
        },
        am: am,
    }
}
