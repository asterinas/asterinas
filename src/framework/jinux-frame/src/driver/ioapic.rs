use acpi::PlatformInfo;

use super::acpi::ACPI_TABLES;
use crate::cell::Cell;
use crate::debug;
use crate::mm::address::phys_to_virt;
use crate::util::recycle_allocator::RecycleAllocator;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref IO_APIC: Cell<IoApic> =
        unsafe { Cell::new(core::mem::MaybeUninit::zeroed().assume_init()) };
}
const IOAPICID: u32 = 0x00;
const IOAPICVER: u32 = 0x01;
const IOAPICARB: u32 = 0x02;

const fn IoApicRedtbl(index: u8) -> u32 {
    0x10 + 2 * index as u32
}

#[derive(Debug)]
#[repr(C)]
struct IoApicRegister {
    address: u32,
    reserved: [u8; 0x10 - 0x04],
    data: u32,
}
impl IoApicRegister {
    pub fn read(self: &mut Self, reg: u32) -> u32 {
        self.address = reg & 0xff;
        self.data
    }

    pub fn write(self: &mut Self, reg: u32, value: u32) {
        self.address = reg & 0xff;
        self.data = value;
    }
}

#[derive(Debug)]
pub struct IoApicEntryHandle {
    index: u8,
}

impl IoApicEntryHandle {
    pub fn read(&mut self) -> u64 {
        let io_apic = IO_APIC.get();
        io_apic.read_irq(self.index)
    }

    pub fn write(&mut self, value: u64) {
        let io_apic = IO_APIC.get();
        io_apic.write_irq(self.index, value);
    }

    pub fn get_index(&self) -> u8 {
        self.index
    }
}

impl Drop for IoApicEntryHandle {
    fn drop(&mut self) {
        let io_apic = IO_APIC.get();
        // mask
        io_apic.write_irq(self.index, 1 << 16);
        io_apic.entry_allocator.dealloc(self.index as usize);
    }
}

#[derive(Debug)]
pub struct IoApic {
    id: u8,
    version: u32,
    max_redirection_entry: u32,
    io_apic_register: &'static mut IoApicRegister,
    entry_allocator: RecycleAllocator,
}

impl IoApic {
    fn read_irq(&mut self, irq_index: u8) -> u64 {
        let low = self.io_apic_register.read(IoApicRedtbl(irq_index)) as u64;
        let high = self.io_apic_register.read(IoApicRedtbl(irq_index) + 1) as u64;
        high << 32 | low
    }

    fn write_irq(&mut self, irq_index: u8, value: u64) {
        let low = value as u32;
        let high = (value >> 32) as u32;
        self.io_apic_register.write(IoApicRedtbl(irq_index), low);
        self.io_apic_register
            .write(IoApicRedtbl(irq_index) + 1, high);
    }

    pub fn allocate_entry(&mut self) -> Option<IoApicEntryHandle> {
        let id = self.entry_allocator.alloc();
        if id == usize::MAX {
            return None;
        }
        Some(IoApicEntryHandle { index: id as u8 })
    }
}

pub fn init() {
    let c = ACPI_TABLES.lock();

    let platform_info = PlatformInfo::new(&*c).unwrap();

    let mut ioapic_address = 0;
    match platform_info.interrupt_model {
        acpi::InterruptModel::Unknown => panic!("not found APIC in ACPI Table"),
        acpi::InterruptModel::Apic(apic) => {
            for io_apic in apic.io_apics.iter() {
                ioapic_address = io_apic.address;
            }
        }
        _ => todo!(),
    }
    if ioapic_address == 0 {
        return;
    }
    let io_apic_register =
        unsafe { &mut *(phys_to_virt(ioapic_address as usize) as *mut IoApicRegister) };

    let id = (read_io_apic(io_apic_register, IOAPICID) & (0xF00_0000) >> 24) as u8;
    let raw_version = read_io_apic(io_apic_register, IOAPICVER);
    let version = raw_version & 0x1ff;
    let max_redirection_entry = ((raw_version & (0xFF_0000)) >> 16) + 1;
    debug!(
        "IOAPIC id: {}, version:{}, max_redirection_entry:{}",
        id, version, max_redirection_entry
    );

    let io_apic = IoApic {
        id,
        version,
        max_redirection_entry,
        io_apic_register,
        entry_allocator: RecycleAllocator::with_start_max(0, max_redirection_entry as usize),
    };

    *IO_APIC.get() = io_apic;
}

fn read_io_apic(io_apic_register: &mut IoApicRegister, reg: u32) -> u32 {
    io_apic_register.address = reg & 0xff;
    io_apic_register.data
}

fn write_io_apic(io_apic_register: &mut IoApicRegister, reg: u32, value: u32) {
    io_apic_register.address = reg & 0xff;
    io_apic_register.data = value;
}
