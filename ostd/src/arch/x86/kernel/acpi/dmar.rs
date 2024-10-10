// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use alloc::vec::Vec;
use core::{fmt::Debug, mem::size_of, slice::Iter};

use acpi::{
    sdt::{SdtHeader, Signature},
    AcpiTable,
};

use super::remapping::{Andd, Atsr, Drhd, Rhsa, Rmrr, Satc, Sidp};
use crate::mm::paddr_to_vaddr;

/// DMA Remapping structure. When IOMMU is enabled, the structure should be present in the ACPI table,
/// and the user can use the DRHD table in this structure to obtain the register base addresses used to configure functions such as IOMMU.
#[derive(Debug)]
pub struct Dmar {
    header: DmarHeader,
    /// Actual size is indicated by `length` in header
    remapping_structures: Vec<Remapping>, // Followed by `n` entries with format `Remapping Structures`
}

/// A DMAR structure contains serval remapping structures. Among these structures,
/// one DRHD must exist, the others must not exist at all.
#[derive(Debug)]
pub enum Remapping {
    Drhd(Drhd),
    Rmrr(Rmrr),
    Atsr(Atsr),
    Rhsa(Rhsa),
    Andd(Andd),
    Satc(Satc),
    Sidp(Sidp),
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
#[allow(clippy::upper_case_acronyms)]
pub enum RemappingType {
    DRHD = 0,
    RMRR = 1,
    ATSR = 2,
    RHSA = 3,
    ANDD = 4,
    SATC = 5,
    SIDP = 6,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct DmarHeader {
    header: SdtHeader,
    host_address_width: u8,
    flags: u8,
    reserved: [u8; 10],
}

unsafe impl AcpiTable for DmarHeader {
    const SIGNATURE: Signature = Signature::DMAR;
    fn header(&self) -> &acpi::sdt::SdtHeader {
        &self.header
    }
}

impl Dmar {
    /// Creates a instance from ACPI table.
    pub fn new() -> Option<Self> {
        if !super::ACPI_TABLES.is_completed() {
            return None;
        }
        let acpi_table_lock = super::ACPI_TABLES.get().unwrap().lock();
        // SAFETY: The DmarHeader is the header for the DMAR structure, it fits all the field described in Intel manual.
        let dmar_mapping = acpi_table_lock.find_table::<DmarHeader>().ok()?;

        let physical_address = dmar_mapping.physical_start();
        let len = dmar_mapping.mapped_length();
        // SAFETY: The target address is the start of the remapping structures,
        // and the length is valid since the value is read from the length field in SDTHeader minus the size of DMAR header.
        let dmar_slice = unsafe {
            core::slice::from_raw_parts_mut(
                paddr_to_vaddr(physical_address + size_of::<DmarHeader>()) as *mut u8,
                len - size_of::<DmarHeader>(),
            )
        };

        let mut remapping_structures = Vec::new();
        let mut index = 0;
        let mut remain_length = len - size_of::<DmarHeader>();
        // SAFETY: Indexes and offsets are strictly followed by the manual.
        unsafe {
            while remain_length > 0 {
                // Common header: type: u16, length: u16
                let length = *dmar_slice[index + 2..index + 4].as_ptr() as usize;
                let typ = *dmar_slice[index..index + 2].as_ptr() as usize;
                let bytes = &&dmar_slice[index..index + length];
                let remapping = match typ {
                    0 => Remapping::Drhd(Drhd::from_bytes(bytes)),
                    1 => Remapping::Rmrr(Rmrr::from_bytes(bytes)),
                    2 => Remapping::Atsr(Atsr::from_bytes(bytes)),
                    3 => Remapping::Rhsa(Rhsa::from_bytes(bytes)),
                    4 => Remapping::Andd(Andd::from_bytes(bytes)),
                    5 => Remapping::Satc(Satc::from_bytes(bytes)),
                    6 => Remapping::Sidp(Sidp::from_bytes(bytes)),
                    _ => {
                        panic!("Unidentified remapping structure type");
                    }
                };
                // let temp = DeviceScope::from_bytes(
                //     &bytes[index as usize..index as usize + length],
                // );
                remapping_structures.push(remapping);
                index += length;
                remain_length -= length;
            }
        }

        Some(Dmar {
            header: *dmar_mapping,
            remapping_structures,
        })
    }

    pub fn remapping_iter(&self) -> Iter<'_, Remapping> {
        self.remapping_structures.iter()
    }
}
