// SPDX-License-Identifier: MPL-2.0

use crate::mm::{paddr_to_vaddr, Paddr};

macro_rules! efi_guid {
    ($a:expr, $b:expr, $c:expr, $d:expr) => {{
        let a = ($a as u32).to_le_bytes(); // u32 -> [u8; 4]
        let b = ($b as u16).to_le_bytes(); // u16 -> [u8; 2]
        let c = ($c as u16).to_le_bytes(); // u16 -> [u8; 2]
        let d = $d;
        EfiGuid {
            b: [
                a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5],
                d[6], d[7],
            ],
        }
    }};
}

/// Reference: <https://github.com/torvalds/linux/blob/master/include/linux/efi.h#L417>
const LINUX_EFI_INITRD_MEDIA_GUID: EfiGuid = efi_guid!(
    0x5568e427,
    0x68fc,
    0x4f3d,
    [0xac, 0x74, 0xca, 0x55, 0x52, 0x31, 0xcc, 0x68]
);

/// Reference: <https://uefi.org/specs/UEFI/2.10/04_EFI_System_Table.html#devicetree-tables>
const DEVICE_TREE_GUID: EfiGuid = efi_guid!(
    0xb1b621d5,
    0xf19c,
    0x41a5,
    [0x83, 0x0b, 0xd9, 0x15, 0x2c, 0x69, 0xaa, 0xe0]
);

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
struct EfiGuid {
    b: [u8; 16],
}

/// Reference: <https://uefi.org/specs/UEFI/2.10/04_EFI_System_Table.html#id4>
#[repr(C)]
struct EfiTableHeader {
    signature: u64,
    revision: u32,
    headersize: u32,
    crc32: u32,
    reserved: u32,
}

/// Reference: <https://uefi.org/specs/UEFI/2.10/04_EFI_System_Table.html#efi-configuration-table>
#[repr(C)]
struct EfiConfigurationTable {
    guid: EfiGuid,
    table: *const core::ffi::c_void,
}

/// Reference: <https://uefi.org/specs/UEFI/2.10/04_EFI_System_Table.html#id6>
#[repr(C)]
pub(super) struct EfiSystemTable {
    hdr: EfiTableHeader,
    fw_vendor: u64, // physical addr of CHAR16*
    fw_revision: u32,
    con_in_handle: u64,
    con_in: *const u64,
    con_out_handle: u64,
    con_out: *const u64,
    stderr_handle: u64,
    stderr_placeholder: u64,
    runtime: *const u64,
    boottime: *const u64,
    nr_tables: u64,
    tables: *const EfiConfigurationTable,
}

// SAFETY: The `EfiSystemTable` structure is only accessed in a read-only manner
// during early EFI initialization. The raw pointers it contains are not written
// to across threads, so it is safe to mark this type as thread-safe.
unsafe impl Sync for EfiSystemTable {}

impl EfiSystemTable {
    fn table(&self, guid: &EfiGuid) -> Option<&EfiConfigurationTable> {
        for i in 0..self.nr_tables as usize {
            let table = unsafe {
                &*(paddr_to_vaddr(self.tables.add(i) as _) as *const EfiConfigurationTable)
            };
            if table.guid == *guid {
                return Some(table);
            }
        }
        None
    }

    pub(super) fn initrd(&self) -> Option<&EfiInitrd> {
        let table = self.table(&LINUX_EFI_INITRD_MEDIA_GUID)?;
        Some(unsafe { &*(paddr_to_vaddr(table.table as _) as *const EfiInitrd) })
    }

    pub(super) fn device_tree(&self) -> Option<Paddr> {
        let table = self.table(&DEVICE_TREE_GUID)?;
        Some(table.table as _)
    }
}

/// Reference: <https://github.com/torvalds/linux/blob/master/include/linux/efi.h#L1327>
#[repr(C)]
pub(super) struct EfiInitrd {
    base: u64,
    size: u64,
}

impl EfiInitrd {
    pub(super) fn range(&self) -> Option<(usize, usize)> {
        if self.size == 0 {
            None
        } else {
            Some((self.base as _, (self.base + self.size) as _))
        }
    }
}
