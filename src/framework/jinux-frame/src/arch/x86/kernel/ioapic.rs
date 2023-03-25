use acpi::PlatformInfo;
use log::info;
use spin::{Mutex, Once};
use x86::apic::ioapic::IoApic;

use super::acpi::ACPI_TABLES;

pub struct IoApicWrapper {
    io_apic: IoApic,
}

impl IoApicWrapper {
    fn new(io_apic: IoApic) -> Self {
        Self { io_apic }
    }

    pub fn disable_all(&mut self) {
        self.io_apic.disable_all()
    }

    pub fn enable(&mut self, irq: u8, cpunum: u8) {
        self.io_apic.enable(irq, cpunum);
    }

    pub fn id(&mut self) -> u8 {
        self.io_apic.id()
    }

    pub fn version(&mut self) -> u8 {
        self.io_apic.version()
    }

    pub fn supported_interrupts(&mut self) -> u8 {
        self.io_apic.supported_interrupts()
    }
}

/// # Safety: The pointer inside the IoApic will not change
unsafe impl Send for IoApicWrapper {}
/// # Safety: The pointer inside the IoApic will not change
unsafe impl Sync for IoApicWrapper {}

pub static IO_APIC: Once<Mutex<IoApicWrapper>> = Once::new();

pub fn init() {
    let c = ACPI_TABLES.get().unwrap().lock();

    let platform_info = PlatformInfo::new(&*c).unwrap();

    let ioapic_address = match platform_info.interrupt_model {
        acpi::InterruptModel::Unknown => panic!("not found APIC in ACPI Table"),
        acpi::InterruptModel::Apic(apic) => {
            apic.io_apics
                .iter()
                .next()
                .expect("There must be at least one IO APIC")
                .address
        }
        _ => {
            panic!("Unknown interrupt model")
        }
    };
    let mut io_apic = unsafe { IoApic::new(crate::vm::paddr_to_vaddr(ioapic_address as usize)) };

    let id = io_apic.id();
    let version = io_apic.version();
    let max_redirection_entry = io_apic.supported_interrupts();
    info!(
        "IOAPIC id: {}, version:{}, max_redirection_entry:{}",
        id, version, max_redirection_entry
    );
    IO_APIC.call_once(|| Mutex::new(IoApicWrapper::new(io_apic)));
}
