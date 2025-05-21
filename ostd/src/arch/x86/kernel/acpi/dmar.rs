// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use alloc::vec::Vec;
use core::{fmt::Debug, slice::Iter};

use acpi::{
    sdt::{SdtHeader, Signature},
    AcpiTable,
};

use super::remapping::{Andd, Atsr, Drhd, Rhsa, Rmrr, Satc, Sidp};

/// DMA Remapping structure.
///
/// When IOMMU is enabled, the structure should be present in the ACPI table, and the user can use
/// the DRHD table in this structure to obtain the register base addresses used to configure
/// functions such IOMMU.
#[derive(Debug)]
pub struct Dmar {
    header: DmarHeader,
    // The actual size is indicated by `length` in `header`.
    // Entries with the format of Remapping Structures are followed.
    remapping_structures: Vec<Remapping>,
}

/// Remapping Structures.
///
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
#[expect(clippy::upper_case_acronyms)]
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

// SAFETY: The `DmarHeader` is the header for the DMAR structure. All its fields are described in
// the Intel manual.
unsafe impl AcpiTable for DmarHeader {
    const SIGNATURE: Signature = Signature::DMAR;
    fn header(&self) -> &acpi::sdt::SdtHeader {
        &self.header
    }
}

impl Dmar {
    /// Creates a instance from ACPI table.
    pub fn new() -> Option<Self> {
        let acpi_table = super::get_acpi_tables()?;

        let dmar_mapping = acpi_table.find_table::<DmarHeader>().ok()?;

        let header = *dmar_mapping;
        // SAFETY: `find_table` returns a region of memory that belongs to the ACPI table. This
        // memory region is valid to read, properly initialized, lives for `'static`, and will
        // never be mutated.
        let slice = unsafe {
            core::slice::from_raw_parts(
                dmar_mapping
                    .virtual_start()
                    .as_ptr()
                    .cast::<u8>()
                    .cast_const(),
                dmar_mapping.mapped_length(),
            )
        };

        let mut index = core::mem::size_of::<DmarHeader>();
        let mut remapping_structures = Vec::new();
        while index != (header.header.length as usize) {
            // CommonHeader { type: u16, length: u16 }
            let typ = u16::from_ne_bytes(slice[index..index + 2].try_into().unwrap());
            let length =
                u16::from_ne_bytes(slice[index + 2..index + 4].try_into().unwrap()) as usize;

            let bytes = &slice[index..index + length];
            let remapping = match typ {
                0 => Remapping::Drhd(Drhd::from_bytes(bytes)),
                1 => Remapping::Rmrr(Rmrr::from_bytes(bytes)),
                2 => Remapping::Atsr(Atsr::from_bytes(bytes)),
                3 => Remapping::Rhsa(Rhsa::from_bytes(bytes)),
                4 => Remapping::Andd(Andd::from_bytes(bytes)),
                5 => Remapping::Satc(Satc::from_bytes(bytes)),
                6 => Remapping::Sidp(Sidp::from_bytes(bytes)),
                _ => {
                    panic!(
                        "the type of the remapping structure is invalid or not supported: {}",
                        typ
                    );
                }
            };
            remapping_structures.push(remapping);

            index += length;
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
