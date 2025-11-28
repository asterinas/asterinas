// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::boot::DEVICE_TREE,
    io::IoMem,
    mm::VmIoOnce,
    power::{inject_poweroff_handler, ExitCode},
};
use spin::Once;

static POWEROFF_REG_AND_VAL: Once<(IoMem, u8)> = Once::new();

fn try_poweroff(_code: ExitCode) {
    // If possible, keep this method panic-free because it may be called by the panic handler.
    if let Some((reg, val)) = POWEROFF_REG_AND_VAL.get() {
        let _ = reg.write_once(0, val);
    }
}

pub(super) fn init() {
    let Some((poweroff_addr, poweroff_value)) = lookup_poweroff_paddr_value() else {
        return;
    };
    let Ok(poweroff_reg) = IoMem::acquire(poweroff_addr..poweroff_addr + size_of::<u8>()) else {
        log::warn!("The poweroff register from syscon-poweroff is not available");
        return;
    };

    POWEROFF_REG_AND_VAL.call_once(move || (poweroff_reg, poweroff_value));
    inject_poweroff_handler(try_poweroff);
}

fn lookup_poweroff_paddr_value() -> Option<(usize, u8)> {
    let device_tree = DEVICE_TREE.get().unwrap();

    let ged = device_tree.find_node("/ged")?;
    if !ged.compatible()?.all().any(|c| c == "syscon") {
        return None;
    }
    let ged_reg_base_address = ged.reg()?.next()?.starting_address as usize;

    let poweroff = device_tree.find_node("/poweroff")?;
    if !poweroff.compatible()?.all().any(|c| c == "syscon-poweroff") {
        return None;
    }
    let poweroff_offset = poweroff.property("offset")?.as_usize()?;
    let poweroff_value = poweroff.property("value")?.as_usize()? as u8;

    Some((ged_reg_base_address + poweroff_offset, poweroff_value))
}
