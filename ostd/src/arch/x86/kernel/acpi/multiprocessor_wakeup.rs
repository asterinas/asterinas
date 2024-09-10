// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;
use core::{marker::PhantomData, mem};

use acpi::{
    fadt::Fadt,
    platform::{
        interrupt::{
            Apic, InterruptModel, InterruptSourceOverride, IoApic, LocalInterruptLine, NmiLine,
            NmiProcessor, NmiSource, Polarity, TriggerMode,
        },
        PmTimer, Processor, ProcessorInfo, ProcessorState,
    },
    sdt::{SdtHeader, Signature},
    AcpiError, AcpiHandler, AcpiTable, AcpiTables, MadtError, PlatformInfo,
};
use bit_field::BitField;

pub struct PlatformInfoWrapper(pub PlatformInfo);

impl PlatformInfoWrapper {
    pub fn new<H>(tables: &AcpiTables<H>) -> Result<PlatformInfoWrapper, AcpiError>
    where
        H: AcpiHandler,
    {
        let fadt = unsafe {
            tables
                .get_sdt::<Fadt>(Signature::FADT)?
                .ok_or(AcpiError::TableMissing(Signature::FADT))?
        };
        let power_profile = fadt.power_profile();

        let madt = unsafe { tables.get_sdt::<Madt>(Signature::MADT)? };
        let (interrupt_model, processor_info) = match madt {
            Some(madt) => madt.parse_interrupt_model()?,
            None => (InterruptModel::Unknown, None),
        };
        let pm_timer = PmTimer::new(&fadt)?;

        Ok(PlatformInfoWrapper(PlatformInfo {
            power_profile,
            interrupt_model,
            processor_info,
            pm_timer,
        }))
    }
}

#[repr(C, packed)]
pub struct Madt {
    header: SdtHeader,
    local_apic_address: u32,
    flags: u32,
}

impl AcpiTable for Madt {
    fn header(&self) -> &SdtHeader {
        &self.header
    }
}

impl Madt {
    pub fn parse_interrupt_model(
        &self,
    ) -> Result<(InterruptModel, Option<ProcessorInfo>), AcpiError> {
        /*
         * We first do a pass through the MADT to determine which interrupt model is being used.
         */
        for entry in self.entries() {
            match entry {
                MadtEntry::LocalApic(_) |
                MadtEntry::LocalX2Apic(_) |
                MadtEntry::IoApic(_) |
                MadtEntry::InterruptSourceOverride(_) |
                MadtEntry::NmiSource(_) |   // TODO: is this one used by more than one model?
                MadtEntry::LocalApicNmi(_) |
                MadtEntry::X2ApicNmi(_) |
                MadtEntry::LocalApicAddressOverride(_) => {
                    return self.parse_apic_model();
                }

                MadtEntry::IoSapic(_) |
                MadtEntry::LocalSapic(_) |
                MadtEntry::PlatformInterruptSource(_) => {
                    unimplemented!();
                }

                MadtEntry::Gicc(_) |
                MadtEntry::Gicd(_) |
                MadtEntry::GicMsiFrame(_) |
                MadtEntry::GicRedistributor(_) |
                MadtEntry::GicInterruptTranslationService(_) => {
                    unimplemented!();
                }

                MadtEntry::MultiprocessorWakeup(_) => ()
            }
        }

        Ok((InterruptModel::Unknown, None))
    }

    fn parse_apic_model(&self) -> Result<(InterruptModel, Option<ProcessorInfo>), AcpiError> {
        let mut local_apic_address = self.local_apic_address as u64;
        let mut io_apic_count = 0;
        let mut iso_count = 0;
        let mut nmi_source_count = 0;
        let mut local_nmi_line_count = 0;
        let mut processor_count = 0usize;

        // Do a pass over the entries so we know how much space we should reserve in the vectors
        for entry in self.entries() {
            match entry {
                MadtEntry::IoApic(_) => io_apic_count += 1,
                MadtEntry::InterruptSourceOverride(_) => iso_count += 1,
                MadtEntry::NmiSource(_) => nmi_source_count += 1,
                MadtEntry::LocalApicNmi(_) => local_nmi_line_count += 1,
                MadtEntry::LocalApic(_) => processor_count += 1,
                _ => (),
            }
        }

        let mut io_apics = Vec::with_capacity(io_apic_count);
        let mut interrupt_source_overrides = Vec::with_capacity(iso_count);
        let mut nmi_sources = Vec::with_capacity(nmi_source_count);
        let mut local_apic_nmi_lines = Vec::with_capacity(local_nmi_line_count);
        let mut boot_processor = None;
        let mut application_processors = Vec::with_capacity(processor_count.saturating_sub(1)); // Subtract one for the BSP

        for entry in self.entries() {
            match entry {
                MadtEntry::LocalApic(entry) => {
                    /*
                     * The first processor is the BSP. Subsequent ones are APs. If we haven't found
                     * the BSP yet, this must be it.
                     */
                    let is_ap = boot_processor.is_some();
                    let is_disabled = !{ entry.flags }.get_bit(0);

                    let state = match (is_ap, is_disabled) {
                        (_, true) => ProcessorState::Disabled,
                        (true, false) => ProcessorState::WaitingForSipi,
                        (false, false) => ProcessorState::Running,
                    };

                    let processor = Processor {
                        processor_uid: entry.processor_id as u32,
                        local_apic_id: entry.apic_id as u32,
                        state,
                        is_ap,
                    };

                    if is_ap {
                        application_processors.push(processor);
                    } else {
                        boot_processor = Some(processor);
                    }
                }

                MadtEntry::LocalX2Apic(entry) => {
                    let is_ap = boot_processor.is_some();
                    let is_disabled = !{ entry.flags }.get_bit(0);

                    let state = match (is_ap, is_disabled) {
                        (_, true) => ProcessorState::Disabled,
                        (true, false) => ProcessorState::WaitingForSipi,
                        (false, false) => ProcessorState::Running,
                    };
                    log::info!("Found X2APIC in MADT!");

                    let processor = Processor {
                        processor_uid: entry.processor_uid,
                        local_apic_id: entry.x2apic_id,
                        state,
                        is_ap,
                    };

                    if is_ap {
                        application_processors.push(processor);
                    } else {
                        boot_processor = Some(processor);
                    }
                }

                MadtEntry::IoApic(entry) => {
                    io_apics.push(IoApic {
                        id: entry.io_apic_id,
                        address: entry.io_apic_address,
                        global_system_interrupt_base: entry.global_system_interrupt_base,
                    });
                }

                MadtEntry::InterruptSourceOverride(entry) => {
                    if entry.bus != 0 {
                        return Err(AcpiError::InvalidMadt(
                            MadtError::InterruptOverrideEntryHasInvalidBus,
                        ));
                    }

                    let (polarity, trigger_mode) = parse_mps_inti_flags(entry.flags)?;

                    interrupt_source_overrides.push(InterruptSourceOverride {
                        isa_source: entry.irq,
                        global_system_interrupt: entry.global_system_interrupt,
                        polarity,
                        trigger_mode,
                    });
                }

                MadtEntry::NmiSource(entry) => {
                    let (polarity, trigger_mode) = parse_mps_inti_flags(entry.flags)?;

                    nmi_sources.push(NmiSource {
                        global_system_interrupt: entry.global_system_interrupt,
                        polarity,
                        trigger_mode,
                    });
                }

                MadtEntry::LocalApicNmi(entry) => local_apic_nmi_lines.push(NmiLine {
                    processor: if entry.processor_id == 0xff {
                        NmiProcessor::All
                    } else {
                        NmiProcessor::ProcessorUid(entry.processor_id as u32)
                    },
                    line: match entry.nmi_line {
                        0 => LocalInterruptLine::Lint0,
                        1 => LocalInterruptLine::Lint1,
                        _ => return Err(AcpiError::InvalidMadt(MadtError::InvalidLocalNmiLine)),
                    },
                }),

                MadtEntry::X2ApicNmi(entry) => local_apic_nmi_lines.push(NmiLine {
                    processor: if entry.processor_uid == 0xffffffff {
                        NmiProcessor::All
                    } else {
                        NmiProcessor::ProcessorUid(entry.processor_uid)
                    },
                    line: match entry.nmi_line {
                        0 => LocalInterruptLine::Lint0,
                        1 => LocalInterruptLine::Lint1,
                        _ => return Err(AcpiError::InvalidMadt(MadtError::InvalidLocalNmiLine)),
                    },
                }),

                MadtEntry::LocalApicAddressOverride(entry) => {
                    local_apic_address = entry.local_apic_address;
                }

                MadtEntry::MultiprocessorWakeup(_) => {}

                _ => {
                    return Err(AcpiError::InvalidMadt(MadtError::UnexpectedEntry));
                }
            }
        }

        Ok((
            InterruptModel::Apic(Apic {
                local_apic_address,
                io_apics,
                local_apic_nmi_lines,
                interrupt_source_overrides,
                nmi_sources,
                also_has_legacy_pics: self.supports_8259(),
            }),
            Some(ProcessorInfo {
                boot_processor: boot_processor.unwrap(),
                application_processors,
            }),
        ))
    }

    pub fn entries(&self) -> MadtEntryIter {
        MadtEntryIter {
            pointer: unsafe { (self as *const Madt as *const u8).add(mem::size_of::<Madt>()) },
            remaining_length: self.header.length - mem::size_of::<Madt>() as u32,
            _phantom: PhantomData,
        }
    }

    pub fn supports_8259(&self) -> bool {
        { self.flags }.get_bit(0)
    }
}

pub struct MadtEntryIter<'a> {
    pointer: *const u8,
    /*
     * The iterator can only have at most `u32::MAX` remaining bytes, because the length of the
     * whole SDT can only be at most `u32::MAX`.
     */
    remaining_length: u32,
    _phantom: PhantomData<&'a ()>,
}

#[allow(dead_code)]
pub enum MadtEntry<'a> {
    LocalApic(&'a LocalApicEntry),
    IoApic(&'a IoApicEntry),
    InterruptSourceOverride(&'a InterruptSourceOverrideEntry),
    NmiSource(&'a NmiSourceEntry),
    LocalApicNmi(&'a LocalApicNmiEntry),
    LocalApicAddressOverride(&'a LocalApicAddressOverrideEntry),
    IoSapic(&'a IoSapicEntry),
    LocalSapic(&'a LocalSapicEntry),
    PlatformInterruptSource(&'a PlatformInterruptSourceEntry),
    LocalX2Apic(&'a LocalX2ApicEntry),
    X2ApicNmi(&'a X2ApicNmiEntry),
    Gicc(&'a GiccEntry),
    Gicd(&'a GicdEntry),
    GicMsiFrame(&'a GicMsiFrameEntry),
    GicRedistributor(&'a GicRedistributorEntry),
    GicInterruptTranslationService(&'a GicInterruptTranslationServiceEntry),
    MultiprocessorWakeup(&'a MultiprocessorWakeupEntry),
}

impl<'a> Iterator for MadtEntryIter<'a> {
    type Item = MadtEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.remaining_length > 0 {
            let entry_pointer = self.pointer;
            let header = unsafe { *(self.pointer as *const EntryHeader) };

            self.pointer = unsafe { self.pointer.offset(header.length as isize) };
            self.remaining_length -= header.length as u32;

            macro_rules! construct_entry {
                ($entry_type:expr,
                 $entry_pointer:expr,
                 $(($value:expr => $variant:path as $type:ty)),*
                ) => {
                    match $entry_type {
                        $(
                            $value => {
                                return Some($variant(unsafe {
                                    &*($entry_pointer as *const $type)
                                }))
                            }
                         )*

                        /*
                         * These entry types are reserved by the ACPI standard. We should skip them
                         * if they appear in a real MADT.
                         */
                        0x11..=0x7f => {}

                        /*
                         * These entry types are reserved for OEM use. Atm, we just skip them too.
                         * TODO: work out if we should ever do anything else here
                         */
                        0x80..=0xff => {}
                    }
                }
            }

            #[rustfmt::skip]
            construct_entry!(
                header.entry_type,
                entry_pointer,
                (0x0 => MadtEntry::LocalApic as LocalApicEntry),
                (0x1 => MadtEntry::IoApic as IoApicEntry),
                (0x2 => MadtEntry::InterruptSourceOverride as InterruptSourceOverrideEntry),
                (0x3 => MadtEntry::NmiSource as NmiSourceEntry),
                (0x4 => MadtEntry::LocalApicNmi as LocalApicNmiEntry),
                (0x5 => MadtEntry::LocalApicAddressOverride as LocalApicAddressOverrideEntry),
                (0x6 => MadtEntry::IoSapic as IoSapicEntry),
                (0x7 => MadtEntry::LocalSapic as LocalSapicEntry),
                (0x8 => MadtEntry::PlatformInterruptSource as PlatformInterruptSourceEntry),
                (0x9 => MadtEntry::LocalX2Apic as LocalX2ApicEntry),
                (0xa => MadtEntry::X2ApicNmi as X2ApicNmiEntry),
                (0xb => MadtEntry::Gicc as GiccEntry),
                (0xc => MadtEntry::Gicd as GicdEntry),
                (0xd => MadtEntry::GicMsiFrame as GicMsiFrameEntry),
                (0xe => MadtEntry::GicRedistributor as GicRedistributorEntry),
                (0xf => MadtEntry::GicInterruptTranslationService as GicInterruptTranslationServiceEntry),
                (0x10 => MadtEntry::MultiprocessorWakeup as MultiprocessorWakeupEntry)
            );
        }

        None
    }
}

#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct EntryHeader {
    entry_type: u8,
    length: u8,
}

#[repr(C, packed)]
pub struct LocalApicEntry {
    header: EntryHeader,
    processor_id: u8,
    apic_id: u8,
    flags: u32,
}

#[repr(C, packed)]
pub struct IoApicEntry {
    header: EntryHeader,
    io_apic_id: u8,
    _reserved: u8,
    io_apic_address: u32,
    global_system_interrupt_base: u32,
}

#[repr(C, packed)]
pub struct InterruptSourceOverrideEntry {
    header: EntryHeader,
    bus: u8, // 0 - ISA bus
    irq: u8, // This is bus-relative
    global_system_interrupt: u32,
    flags: u16,
}

#[repr(C, packed)]
pub struct NmiSourceEntry {
    header: EntryHeader,
    flags: u16,
    global_system_interrupt: u32,
}

#[repr(C, packed)]
pub struct LocalApicNmiEntry {
    header: EntryHeader,
    processor_id: u8,
    flags: u16,
    nmi_line: u8, // Describes which LINTn is the NMI connected to
}

#[repr(C, packed)]
pub struct LocalApicAddressOverrideEntry {
    header: EntryHeader,
    _reserved: u16,
    local_apic_address: u64,
}

/// If this entry is present, the system has an I/O SAPIC, which must be used instead of the I/O
/// APIC.
#[repr(C, packed)]
pub struct IoSapicEntry {
    header: EntryHeader,
    io_apic_id: u8,
    _reserved: u8,
    global_system_interrupt_base: u32,
    io_sapic_address: u64,
}

#[repr(C, packed)]
pub struct LocalSapicEntry {
    header: EntryHeader,
    processor_id: u8,
    local_sapic_id: u8,
    local_sapic_eid: u8,
    _reserved: [u8; 3],
    flags: u32,
    processor_uid: u32,

    /// This string can be used to associate this local SAPIC to a processor defined in the
    /// namespace when the `_UID` object is a string. It is a null-terminated ASCII string, and so
    /// this field will be `'\0'` if the string is not present, otherwise it extends from the
    /// address of this field.
    processor_uid_string: u8,
}

#[repr(C, packed)]
pub struct PlatformInterruptSourceEntry {
    header: EntryHeader,
    flags: u16,
    interrupt_type: u8,
    processor_id: u8,
    processor_eid: u8,
    io_sapic_vector: u8,
    global_system_interrupt: u32,
    platform_interrupt_source_flags: u32,
}

#[repr(C, packed)]
pub struct LocalX2ApicEntry {
    header: EntryHeader,
    _reserved: u16,
    x2apic_id: u32,
    flags: u32,
    processor_uid: u32,
}

#[repr(C, packed)]
pub struct X2ApicNmiEntry {
    header: EntryHeader,
    flags: u16,
    processor_uid: u32,
    nmi_line: u8,
    _reserved: [u8; 3],
}

/// This field will appear for ARM processors that support ACPI and use the Generic Interrupt
/// Controller. In the GICC interrupt model, each logical process has a Processor Device object in
/// the namespace, and uses this structure to convey its GIC information.
#[repr(C, packed)]
pub struct GiccEntry {
    header: EntryHeader,
    _reserved1: u16,
    cpu_interface_number: u32,
    processor_uid: u32,
    flags: u32,
    parking_protocol_version: u32,
    performance_interrupt_gsiv: u32,
    parked_address: u64,
    gic_registers_address: u64,
    gic_control_block_address: u64,
    vgic_maintenance_interrupt: u32,
    gicr_base_address: u64,
    mpidr: u64,
    processor_power_efficiency_class: u8,
    _reserved2: [u8; 3],
}

#[repr(C, packed)]
pub struct GicdEntry {
    header: EntryHeader,
    _reserved1: u16,
    gic_id: u32,
    physical_base_address: u64,
    system_vector_base: u32,

    /// The GIC version
    ///     0x00: Fall back to hardware discovery
    ///     0x01: GICv1
    ///     0x02: GICv2
    ///     0x03: GICv3
    ///     0x04: GICv4
    ///     0x05-0xff: Reserved for future use
    gic_version: u8,
    _reserved2: [u8; 3],
}

#[repr(C, packed)]
pub struct GicMsiFrameEntry {
    header: EntryHeader,
    _reserved: u16,
    frame_id: u32,
    physical_base_address: u64,
    flags: u32,
    spi_count: u16,
    spi_base: u16,
}

#[repr(C, packed)]
pub struct GicRedistributorEntry {
    header: EntryHeader,
    _reserved: u16,
    discovery_range_base_address: u64,
    discovery_range_length: u32,
}

#[repr(C, packed)]
pub struct GicInterruptTranslationServiceEntry {
    header: EntryHeader,
    _reserved1: u16,
    id: u32,
    physical_base_address: u64,
    _reserved2: u32,
}

#[repr(C, packed)]
pub struct MultiprocessorWakeupEntry {
    header: EntryHeader,
    mailbox_version: u16,
    _reserved: u32,
    mailbox_address: u64,
}

fn parse_mps_inti_flags(flags: u16) -> Result<(Polarity, TriggerMode), AcpiError> {
    let polarity = match flags.get_bits(0..2) {
        0b00 => Polarity::SameAsBus,
        0b01 => Polarity::ActiveHigh,
        0b11 => Polarity::ActiveLow,
        _ => return Err(AcpiError::InvalidMadt(MadtError::MpsIntiInvalidPolarity)),
    };

    let trigger_mode = match flags.get_bits(2..4) {
        0b00 => TriggerMode::SameAsBus,
        0b01 => TriggerMode::Edge,
        0b11 => TriggerMode::Level,
        _ => return Err(AcpiError::InvalidMadt(MadtError::MpsIntiInvalidTriggerMode)),
    };
    Ok((polarity, trigger_mode))
}

