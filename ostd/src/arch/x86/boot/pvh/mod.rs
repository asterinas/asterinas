// SPDX-License-Identifier: MPL-2.0

//! The PVH boot protocol supporting module.
//!
//! PVH is the direct boot protocol implemented by most modern virtual machine
//! monitors (e.g., Cloud Hypervisor, Firecracker, QEMU). The VMM enters the
//! kernel in 32-bit protected mode with paging disabled and passes the
//! physical address of an [`HvmStartInfo`] structure in `EBX`. The entry point
//! itself is advertised through the PVH ELF note (see `note.S`).
//!
//! Reference:
//! <https://xenbits.xen.org/docs/unstable/misc/pvh.html>
//! <https://github.com/xen-project/xen/blob/staging-4.19/xen/include/public/arch-x86/hvm/start_info.h>

use core::ffi::CStr;

use int_to_c_enum::TryFromInt;

use super::ToEarlyBootInfo;
use crate::{
    boot::{
        BootloaderAcpiArg, BootloaderFramebufferArg, EARLY_INFO,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
        start_kernel,
    },
    mm::{Paddr, kspace::paddr_to_vaddr},
};

#[cfg(feature = "pvh_boot")]
core::arch::global_asm!(include_str!("note.S"));

/// The magic value of [`HvmStartInfo::magic`], which is "xEn3" in ASCII.
const HVM_START_MAGIC_VALUE: u32 = 0x336e_c578;

/// The start-of-day structure that a PVH-capable VMM passes to the guest.
#[repr(C)]
struct HvmStartInfo {
    /// Magic value; must equal [`HVM_START_MAGIC_VALUE`].
    magic: u32,
    /// Structure version. The memory-map fields are only present in version 1
    /// and later.
    version: u32,
    /// Flags; currently unused.
    flags: u32,
    /// Number of entries in the module list.
    nr_modules: u32,
    /// Physical address of the module list (an array of [`HvmModlistEntry`]).
    modlist_paddr: u64,
    /// Physical address of the NUL-terminated kernel command line.
    cmdline_paddr: u64,
    /// Physical address of the RSDP.
    rsdp_paddr: u64,
    // The following fields are only present in version 1 and later.
    /// Physical address of the memory map (an array of [`HvmMemmapTableEntry`]).
    memmap_paddr: u64,
    /// Number of entries in the memory map.
    memmap_entries: u32,
    /// Reserved.
    _reserved: u32,
}

/// An entry of the module list pointed to by [`HvmStartInfo::modlist_paddr`].
#[repr(C)]
struct HvmModlistEntry {
    /// Physical address of the module.
    paddr: u64,
    /// Size of the module in bytes.
    size: u64,
    /// Physical address of the module's NUL-terminated command line.
    cmdline_paddr: u64,
    /// Reserved.
    _reserved: u64,
}

/// An entry of the memory map pointed to by [`HvmStartInfo::memmap_paddr`].
#[repr(C)]
struct HvmMemmapTableEntry {
    /// Start address of the region.
    addr: u64,
    /// Size of the region in bytes.
    size: u64,
    /// Region type; see [`HvmMemmapType`].
    typ: u32,
    /// Reserved.
    _reserved: u32,
}

/// The type of a PVH memory map entry.
#[repr(u32)]
#[derive(Debug, TryFromInt)]
enum HvmMemmapType {
    /// Usable RAM.
    Ram = 1,
    /// Memory reserved by the VMM.
    Reserved = 2,
    /// ACPI tables; the memory can be reclaimed once they are parsed.
    Acpi = 3,
    /// Non-volatile memory that must be preserved across sleep.
    Nvs = 4,
    /// Memory that cannot be used.
    Unusable = 5,
    /// Memory that has been disabled.
    Disabled = 6,
    /// Persistent memory.
    Pmem = 7,
}

impl From<HvmMemmapType> for MemoryRegionType {
    fn from(typ: HvmMemmapType) -> Self {
        match typ {
            HvmMemmapType::Ram => Self::Usable,
            HvmMemmapType::Reserved | HvmMemmapType::Disabled | HvmMemmapType::Pmem => {
                Self::Reserved
            }
            HvmMemmapType::Acpi => Self::Reclaimable,
            HvmMemmapType::Nvs => Self::NonVolatileSleep,
            HvmMemmapType::Unusable => Self::BadMemory,
        }
    }
}

fn parse_memory_region_type(typ: u32) -> MemoryRegionType {
    HvmMemmapType::try_from(typ)
        .map(MemoryRegionType::from)
        .unwrap_or(MemoryRegionType::Reserved)
}

impl HvmStartInfo {
    fn modules(&self) -> &[HvmModlistEntry] {
        if self.nr_modules == 0 || self.modlist_paddr == 0 {
            return &[];
        }

        // SAFETY: `modlist_paddr` points to an array of at least `nr_modules`
        // valid entries by the contract with the VMM.
        unsafe {
            core::slice::from_raw_parts(
                paddr_to_vaddr(self.modlist_paddr as Paddr) as *const HvmModlistEntry,
                self.nr_modules as usize,
            )
        }
    }

    fn memory_map(&self) -> Option<&[HvmMemmapTableEntry]> {
        if self.version < 1 || self.memmap_paddr == 0 {
            return None;
        }

        // SAFETY: `memmap_paddr` points to an array of `memmap_entries` valid
        // entries by the contract with the VMM.
        Some(unsafe {
            core::slice::from_raw_parts(
                paddr_to_vaddr(self.memmap_paddr as Paddr) as *const HvmMemmapTableEntry,
                self.memmap_entries as usize,
            )
        })
    }
}

impl ToEarlyBootInfo for HvmStartInfo {
    fn bootloader_name(&self) -> &'static str {
        "PVH-capable VMM"
    }

    fn kernel_commandline(&self) -> Option<&'static str> {
        if self.cmdline_paddr == 0 {
            return None;
        }

        // SAFETY:
        // 1. The command line is safe to read because of the contract with the VMM.
        // 2. We reserve the command-line region in `finish_memory_regions`, so it will live as an
        //    immutable reference for `'static`.
        let cmdline =
            unsafe { CStr::from_ptr(paddr_to_vaddr(self.cmdline_paddr as Paddr) as *const _) };

        cmdline.to_str().ok()
    }

    fn initramfs(&self) -> Option<&'static [u8]> {
        // The PVH protocol does not tag module kinds, so the first module is
        // treated as the initramfs by convention.
        let module = self.modules().first()?;

        if module.paddr == 0 || module.size == 0 {
            return None;
        }

        // SAFETY:
        // 1. The initramfs is safe to read because of the contract with the loader.
        // 2. We reserve the initramfs region in `memory_regions`, so it will live as an
        //    immutable reference for `'static`.
        Some(unsafe {
            core::slice::from_raw_parts(
                paddr_to_vaddr(module.paddr as Paddr) as *const u8,
                module.size as usize,
            )
        })
    }

    fn acpi_arg(&self) -> BootloaderAcpiArg {
        if self.rsdp_paddr == 0 {
            BootloaderAcpiArg::ScanBios
        } else {
            BootloaderAcpiArg::Rsdp(
                self.rsdp_paddr
                    .try_into()
                    .expect("RSDP address overflowed!"),
            )
        }
    }

    fn framebuffer_arg(&self) -> Option<BootloaderFramebufferArg> {
        // PVH does not provide a bootloader framebuffer.
        None
    }

    fn memory_regions(
        &self,
        initramfs: Option<&'static [u8]>,
        kernel_cmdline: Option<&'static str>,
        framebuffer_arg: Option<BootloaderFramebufferArg>,
    ) -> MemoryRegionArray {
        let mut regions = MemoryRegionArray::new();

        let acpi_root_table_address = if self.rsdp_paddr == 0 {
            super::find_acpi_root_table_address()
        } else {
            None
        };

        if let Some(memmap) = self.memory_map() {
            for entry in memmap {
                let base = entry.addr.try_into().unwrap();
                let len = entry.size.try_into().unwrap();
                let typ = super::effective_region_type(
                    parse_memory_region_type(entry.typ),
                    base,
                    len,
                    acpi_root_table_address,
                );

                regions.push(MemoryRegion::new(base, len, typ)).unwrap();
            }
        }

        super::finish_memory_regions(regions, framebuffer_arg, initramfs, kernel_cmdline)
    }
}

/// The entry point of the Rust code portion of Asterinas (with PVH parameters).
///
/// # Safety
///
/// - This function must be called only once at a proper timing in the BSP's boot assembly code.
/// - The caller must follow C calling conventions and put the right arguments in registers.
/// - If this function is called, entry points of other boot protocols must never be called.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "sysv64" fn __pvh_entry(start_info_ptr: *const HvmStartInfo) -> ! {
    // SAFETY: We get the start-of-day structure from the VMM, so by contract the pointer is valid
    // and the underlying memory is initialized.
    let start_info = unsafe { &*start_info_ptr };

    assert_eq!(start_info.magic, HVM_START_MAGIC_VALUE);

    EARLY_INFO.call_once(|| start_info.to_early_boot_info());

    // SAFETY: The safety is guaranteed by the safety preconditions and the fact that we call it
    // once after setting up necessary resources.
    unsafe { start_kernel() };
}
