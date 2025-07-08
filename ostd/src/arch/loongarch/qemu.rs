// SPDX-License-Identifier: MPL-2.0

//! Providing the ability to exit QEMU and return a value as debug result.

use crate::arch::{boot::DEVICE_TREE, mm::paddr_to_daddr};

/// The exit code of QEMU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuExitCode {
    /// The code that indicates a successful exit.
    Success,
    /// The code that indicates a failed exit.
    Failed,
}

/// Exits QEMU with the given exit code.
//  FIXME: Support the transfer of the exit code to QEMU and multiple platforms.
pub fn exit_qemu(_exit_code: QemuExitCode) -> ! {
    let (poweroff_addr, poweroff_value) = lookup_poweroff_paddr_value().unwrap();
    let poweroff_daddr = paddr_to_daddr(poweroff_addr) as *mut u8;

    // SAFETY: It is safe because the poweroff address is acquired from the device tree,
    // and be mapped in DMW2.
    unsafe {
        core::ptr::write_volatile(poweroff_daddr, poweroff_value);
    }
    unreachable!("Qemu does not exit");
}

// FIXME: We should reserve the address region in `io_mem_allocator`.
fn lookup_poweroff_paddr_value() -> Option<(usize, u8)> {
    let device_tree = DEVICE_TREE.get().unwrap();

    let ged = device_tree.find_node("/ged")?;
    if !ged.compatible()?.all().any(|c| c == "syscon") {
        return None;
    }
    let ged_reg_base_address = ged.reg()?.next()?.starting_address as usize;

    let poweroff = device_tree.find_node("/poweroff").unwrap();
    if !poweroff.compatible()?.all().any(|c| c == "syscon-poweroff") {
        return None;
    }
    let poweroff_offset = poweroff.property("offset")?.as_usize()?;
    let poweroff_value = poweroff.property("value")?.as_usize()? as u8;

    Some((ged_reg_base_address + poweroff_offset, poweroff_value))
}
