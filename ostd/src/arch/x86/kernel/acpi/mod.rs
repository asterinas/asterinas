// SPDX-License-Identifier: MPL-2.0

// Set this module's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "acpi: "
    };
}

pub(in crate::arch) mod dmar;
pub(in crate::arch) mod remapping;

use core::{num::NonZeroU8, ptr::NonNull};

use acpi::{
    AcpiHandler, AcpiTable, AcpiTables,
    address::AddressSpace,
    fadt::{Fadt, IaPcBootArchFlags},
    mcfg::Mcfg,
    rsdp::Rsdp,
};
use spin::Once;

use crate::{
    boot::{self, BootloaderAcpiArg},
    info,
    mm::paddr_to_vaddr,
    warn,
};

#[derive(Clone, Debug)]
pub(crate) struct AcpiMemoryHandler {}

impl AcpiHandler for AcpiMemoryHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> acpi::PhysicalMapping<Self, T> {
        let virtual_address = NonNull::new(paddr_to_vaddr(physical_address) as *mut T).unwrap();

        // SAFETY: The caller should guarantee that `physical_address..physical_address + size` is
        // part of the ACPI table. Then the memory region is mapped to `virtual_address` and is
        // valid for read and immutable dereferencing.
        // FIXME: The caller guarantee only holds if we trust the hardware to provide a valid ACPI
        // table. Otherwise, if the table is corrupted, it may reference arbitrary memory regions.
        unsafe {
            acpi::PhysicalMapping::new(physical_address, virtual_address, size, size, self.clone())
        }
    }

    fn unmap_physical_region<T>(_region: &acpi::PhysicalMapping<Self, T>) {}
}

struct SyncAcpiTables(Option<AcpiTables<AcpiMemoryHandler>>);

// SAFETY: This relies on the current implementation of `AcpiTables`,
// which provides thread-safe access to read-only ACPI table data,
// so `Sync` is sound for the wrapper.
// FIXME: It depends on implementation details of `AcpiTables`, which should be avoided.
unsafe impl Sync for SyncAcpiTables {}

static ACPI_TABLES: Once<SyncAcpiTables> = Once::new();

pub(crate) fn get_acpi_tables() -> Option<&'static AcpiTables<AcpiMemoryHandler>> {
    let acpi_tables = ACPI_TABLES.call_once(|| {
        let acpi_tables = match boot::EARLY_INFO.get().unwrap().acpi_arg {
            BootloaderAcpiArg::Rsdp(addr) => unsafe {
                AcpiTables::from_rsdp(AcpiMemoryHandler {}, addr).unwrap()
            },
            BootloaderAcpiArg::Rsdt(addr) => unsafe {
                AcpiTables::from_rsdt(AcpiMemoryHandler {}, 0, addr).unwrap()
            },
            BootloaderAcpiArg::Xsdt(addr) => unsafe {
                AcpiTables::from_rsdt(AcpiMemoryHandler {}, 1, addr).unwrap()
            },
            BootloaderAcpiArg::ScanBios => {
                // SAFETY: The selected boot path permits legacy BIOS RSDP scanning.
                let rsdp = unsafe { Rsdp::search_for_on_bios(AcpiMemoryHandler {}) };
                match rsdp {
                    Ok(map) => unsafe {
                        AcpiTables::from_rsdp(AcpiMemoryHandler {}, map.physical_start()).unwrap()
                    },
                    Err(_) => {
                        warn!("ACPI info not found!");
                        return SyncAcpiTables(None);
                    }
                }
            }
            BootloaderAcpiArg::NotProvided => {
                warn!("ACPI info not found!");
                return SyncAcpiTables(None);
            }
        };

        SyncAcpiTables(Some(acpi_tables))
    });

    acpi_tables.0.as_ref()
}

/// The platform information provided by the ACPI tables.
///
/// Currently, this structure contains only a limited set of fields, far fewer than those in all
/// ACPI tables. However, the goal is to expand it properly to keep the simplicity of the OSTD code
/// while enabling OSTD users to safely retrieve information from the ACPI tables.
#[derive(Debug)]
pub struct AcpiInfo {
    /// The RTC CMOS RAM index to the century of data value; the "CENTURY" field in the FADT.
    pub century_register: Option<NonZeroU8>,
    /// IA-PC Boot Architecture Flags; the "IAPC_BOOT_ARCH" field in the FADT.
    pub boot_flags: Option<IaPcBootArchFlags>,
    /// An I/O port to reset the machine by writing the specified value.
    pub reset_port_and_val: Option<(u16, u8)>,
    /// A memory region that is stolen for PCI configuration space.
    pub pci_ecam_region: Option<PciEcamRegion>,
    /// Fixed-hardware power management registers (I/O space only).
    pub pm: Option<AcpiPmInfo>,
    /// The OEM ID of the FADT, e.g. `BOCHS ` for QEMU, `AMAZON` for EC2 Nitro.
    pub oem_id: Option<[u8; 6]>,
}

/// The PM1 event/control register blocks from the FADT plus the S5 sleep
/// type from the DSDT, enough to notice an ACPI power button press and to
/// enter soft-off without an AML interpreter.
#[derive(Clone, Copy, Debug)]
pub struct AcpiPmInfo {
    /// PM1a event block: status register at `+0`, enable register at
    /// `+pm1_event_len/2`.
    pub pm1a_event_port: u16,
    /// PM1b event block, if the platform has one.
    pub pm1b_event_port: Option<u16>,
    /// Length of a PM1 event block in bytes (status + enable).
    pub pm1_event_len: u8,
    /// PM1a control register.
    pub pm1a_control_port: u16,
    /// PM1b control register, if the platform has one.
    pub pm1b_control_port: Option<u16>,
    /// `(SLP_TYPa, SLP_TYPb)` of the `_S5` package, if the DSDT has one.
    pub s5_sleep_type: Option<(u8, u8)>,
}

/// A memory region that is stolen for PCI configuration space.
#[derive(Debug)]
pub struct PciEcamRegion {
    /// The base address of the memory region.
    pub base_address: u64,
    /// The start of the bus number.
    pub bus_start: u8,
    /// The end of the bus number.
    pub bus_end: u8,
}

/// The [`AcpiInfo`] singleton.
pub static ACPI_INFO: Once<AcpiInfo> = Once::new();

pub(in crate::arch) fn init() {
    let mut acpi_info = AcpiInfo {
        century_register: None,
        boot_flags: None,
        reset_port_and_val: None,
        pci_ecam_region: None,
        pm: None,
        oem_id: None,
    };

    let Some(acpi_tables) = get_acpi_tables() else {
        ACPI_INFO.call_once(|| acpi_info);
        return;
    };

    if let Ok(fadt) = acpi_tables.find_table::<Fadt>() {
        // A zero means that the century register does not exist.
        acpi_info.century_register = NonZeroU8::new(fadt.century);
        acpi_info.boot_flags = Some(fadt.iapc_boot_arch);
        if let Ok(reset_reg) = fadt.reset_register()
            && reset_reg.address_space == AddressSpace::SystemIo
            && let Ok(reset_port) = reset_reg.address.try_into()
        {
            acpi_info.reset_port_and_val = Some((reset_port, fadt.reset_value));
        }
        acpi_info.pm = collect_pm_info(&fadt, acpi_tables);
        acpi_info.oem_id = Some(fadt.header().oem_id);
    };

    if let Ok(mcfg) = acpi_tables.find_table::<Mcfg>()
        // TODO: Support multiple PCIe segment groups instead of assuming only one
        // PCIe segment group is in use.
        && let Some(mcfg_entry) = mcfg.entries().first()
    {
        acpi_info.pci_ecam_region = Some(PciEcamRegion {
            base_address: mcfg_entry.base_address,
            bus_start: mcfg_entry.bus_number_start,
            bus_end: mcfg_entry.bus_number_end,
        });
    }

    info!("Collected information {:?}", acpi_info);

    ACPI_INFO.call_once(|| acpi_info);
}

fn io_port(addr: acpi::address::GenericAddress) -> Option<u16> {
    if addr.address_space != AddressSpace::SystemIo || addr.address == 0 {
        return None;
    }
    addr.address.try_into().ok()
}

fn collect_pm_info(fadt: &Fadt, acpi_tables: &AcpiTables<AcpiMemoryHandler>) -> Option<AcpiPmInfo> {
    let pm1a_event = fadt.pm1a_event_block().ok()?;
    let pm1a_event_port = io_port(pm1a_event)?;
    let pm1a_control_port = io_port(fadt.pm1a_control_block().ok()?)?;
    let pm1b_event_port = fadt.pm1b_event_block().ok().flatten().and_then(io_port);
    let pm1b_control_port = fadt.pm1b_control_block().ok().flatten().and_then(io_port);
    let pm1_event_len = (pm1a_event.bit_width / 8).max(4);

    // `_S5` is usually in the DSDT; EC2 Nitro puts it in an SSDT.
    let aml_tables = acpi_tables
        .dsdt()
        .ok()
        .into_iter()
        .chain(acpi_tables.ssdts());
    let mut s5_sleep_type = None;
    let mut total = 0usize;
    for table in aml_tables {
        // SAFETY: Firmware tables in the linearly mapped low physical memory; read only.
        let bytes = unsafe {
            core::slice::from_raw_parts(
                paddr_to_vaddr(table.address) as *const u8,
                table.length as usize,
            )
        };
        total += bytes.len();
        if let Some(found) = find_s5_sleep_type(bytes) {
            s5_sleep_type = Some(found);
            break;
        }
        if let Some(at) = bytes.windows(4).position(|w| w == b"_S5_") {
            // Show the raw encoding so the parser can be taught the next variant.
            let lo = at.saturating_sub(4);
            let hi = (at + 16).min(bytes.len());
            crate::early_println!(
                "[kernel] acpi: _S5_ found but not parsed; bytes {:02x?}",
                &bytes[lo..hi]
            );
        }
    }
    if s5_sleep_type.is_none() {
        crate::early_println!(
            "[kernel] acpi: no _S5_ in the DSDT/SSDTs ({} bytes of AML)",
            total
        );
    }

    Some(AcpiPmInfo {
        pm1a_event_port,
        pm1b_event_port,
        pm1_event_len,
        pm1a_control_port,
        pm1b_control_port,
        s5_sleep_type,
    })
}

/// Finds `Name (_S5_, Package { SLP_TYPa, SLP_TYPb, ... })` in raw AML.
///
/// This is the one AML object a kernel needs for soft-off, and its encoding
/// is fixed enough (`NameOp` `_S5_` `PackageOp` `PkgLength` `NumElements`
/// then integer constants) that a byte scan replaces an interpreter. We
/// accept `Zero`, `One`, `ByteConst`, `WordConst` and `DWordConst` elements,
/// which covers the firmware seen so far (QEMU: `{0, 0}`; most PCs and EC2
/// Nitro: `{5, 5}` or `{7, 7}`).
fn find_s5_sleep_type(aml: &[u8]) -> Option<(u8, u8)> {
    const NAME_OP: u8 = 0x08;
    const ROOT_CHAR: u8 = 0x5C;
    const PACKAGE_OP: u8 = 0x12;
    // `Name (_S5_, Package ...)` or `Name (\_S5_, Package ...)`: the name may
    // carry a root prefix, so match on the name + PackageOp and check what
    // precedes it.
    let tail = [b'_', b'S', b'5', b'_', PACKAGE_OP];
    let mut pos = 0;
    let start = loop {
        let rel = aml[pos..].windows(tail.len()).position(|w| w == tail)?;
        let at = pos + rel;
        let preceded_by_name_op = at >= 1 && aml[at - 1] == NAME_OP
            || at >= 2 && aml[at - 1] == ROOT_CHAR && aml[at - 2] == NAME_OP;
        if preceded_by_name_op {
            break at + tail.len();
        }
        pos = at + 1;
    };
    let mut i = start;
    // PkgLength: the top two bits of the first byte give the number of extra bytes.
    let extra = (aml.get(i)? >> 6) as usize;
    i += 1 + extra;
    let _num_elements = *aml.get(i)?;
    i += 1;
    let read_int = |i: &mut usize| -> Option<u8> {
        let op = *aml.get(*i)?;
        *i += 1;
        let v = match op {
            0x00 => 0,
            0x01 => 1,
            0x0A => {
                let v = *aml.get(*i)?;
                *i += 1;
                v
            }
            0x0B => {
                let v = *aml.get(*i)?;
                *i += 2;
                v
            }
            0x0C => {
                let v = *aml.get(*i)?;
                *i += 4;
                v
            }
            _ => return None,
        };
        Some(v & 0x7)
    };
    let a = read_int(&mut i)?;
    let b = read_int(&mut i).unwrap_or(a);
    Some((a, b))
}
