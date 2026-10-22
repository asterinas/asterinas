// SPDX-License-Identifier: MPL-2.0

//! Power management.

use fdt::{Fdt, node::FdtNode};
use spin::Once;

use crate::{
    arch::boot,
    io::IoMem,
    mm::{HasPaddr, HasSize, VmIoOnce},
    power::ExitCode,
};

static POWEROFF: Once<PowerAction> = Once::new();
static RESTART: Once<PowerAction> = Once::new();

/// Attempts to power off the system using an architecture-specific mechanism.
///
/// On LoongArch, this function attempts to power off the system if a supported `syscon-poweroff`
/// device tree node exists. Otherwise, it does nothing and returns.
pub fn try_poweroff(_code: ExitCode) {
    if let Some(action) = POWEROFF.get() {
        let _ = action.trigger();
    }
}

/// Attempts to restart the system using an architecture-specific mechanism.
///
/// On LoongArch, this function attempts to restart the system if a supported `syscon-reboot` device
/// tree node exists. Otherwise, it does nothing and returns.
pub fn try_restart(_code: ExitCode) {
    if let Some(action) = RESTART.get() {
        let _ = action.trigger();
    }
}

pub(super) fn init() {
    let Some(device_tree) = boot::DEVICE_TREE.get() else {
        return;
    };

    let poweroff = find_action(device_tree, "syscon-poweroff", None);
    let restart = find_action(device_tree, "syscon-reboot", poweroff.as_ref());

    if let Some(action) = poweroff {
        POWEROFF.call_once(|| action);
    }
    if let Some(action) = restart {
        RESTART.call_once(|| action);
    }
}

fn find_action(
    device_tree: &Fdt,
    compatible: &str,
    shared: Option<&PowerAction>,
) -> Option<PowerAction> {
    device_tree
        .all_nodes()
        .filter(is_available)
        .find_map(|parent| {
            parent
                .children()
                .filter(is_available)
                .filter(|node| {
                    node.compatible()
                        .is_some_and(|c| c.all().any(|c| c == compatible))
                })
                .find_map(|node| PowerAction::parse(device_tree, node, parent, shared))
        })
}

fn is_available(node: &FdtNode) -> bool {
    match node.property("status") {
        None => true,
        Some(status) => matches!(status.as_str(), Some("okay" | "ok")),
    }
}

struct PowerAction {
    register: IoMem,
    io_width: usize,
    value: u32,
    mask: u32,
}

impl PowerAction {
    fn parse(
        device_tree: &Fdt,
        node: FdtNode,
        parent: FdtNode,
        shared: Option<&Self>,
    ) -> Option<Self> {
        // References: <https://www.kernel.org/doc/Documentation/devicetree/bindings/power/reset/syscon-poweroff.yaml>
        // <https://www.kernel.org/doc/Documentation/devicetree/bindings/power/reset/syscon-reboot.yaml>
        let syscon = match node.property("regmap") {
            Some(phandle) => device_tree.find_phandle(phandle.as_usize()?.try_into().ok()?)?,
            None => parent,
        };
        if !is_available(&syscon) || !syscon.compatible()?.all().any(|c| c == "syscon") {
            return None;
        }

        let io_width = match syscon.property("reg-io-width") {
            Some(width) => width.as_usize()?,
            None => 4,
        };
        if !matches!(io_width, 1 | 2 | 4) {
            return None;
        }

        let (value, mask): (u32, u32) = match (node.property("value"), node.property("mask")) {
            (Some(value), mask) => (
                value.as_usize()?.try_into().ok()?,
                match mask {
                    Some(mask) => mask.as_usize()?.try_into().ok()?,
                    None => u32::MAX,
                },
            ),
            // The legacy mask-only binding uses the mask as a full-register value.
            (None, Some(mask)) => (mask.as_usize()?.try_into().ok()?, u32::MAX),
            _ => return None,
        };
        let register_mask = u32::MAX >> (8 * (4 - io_width));
        if value & !register_mask != 0 {
            return None;
        }

        let (value, mask) = if syscon.property("big-endian").is_some() {
            match io_width {
                2 => (
                    (value as u16).swap_bytes() as u32,
                    (mask as u16).swap_bytes() as u32,
                ),
                4 => (value.swap_bytes(), mask.swap_bytes()),
                _ => (value, mask),
            }
        } else {
            (value, mask)
        };

        let reg = syscon.reg()?.next()?;
        let offset = node.property("offset")?.as_usize()?;
        if !offset.is_multiple_of(io_width) || offset.checked_add(io_width)? > reg.size? {
            return None;
        }
        let start = (reg.starting_address as usize).checked_add(offset)?;
        let end = start.checked_add(io_width)?;
        if !start.is_multiple_of(io_width) {
            return None;
        }

        let register = match shared {
            Some(action)
                if action.register.paddr() == start && action.register.size() == io_width =>
            {
                action.register.clone()
            }
            _ => IoMem::acquire(start..end).ok()?,
        };

        Some(Self {
            register,
            io_width,
            value,
            mask,
        })
    }

    fn trigger(&self) -> crate::Result<()> {
        let register_mask = u32::MAX >> (8 * (4 - self.io_width));
        // A full-register write does not require reading a potentially write-only register.
        let old_value = if self.mask & register_mask == register_mask {
            0
        } else {
            match self.io_width {
                1 => self.register.read_once::<u8>(0)? as u32,
                2 => self.register.read_once::<u16>(0)? as u32,
                _ => self.register.read_once::<u32>(0)?,
            }
        };
        let value = (old_value & !self.mask) | (self.value & self.mask);

        match self.io_width {
            1 => self.register.write_once(0, &(value as u8)),
            2 => self.register.write_once(0, &(value as u16)),
            _ => self.register.write_once(0, &value),
        }
    }
}
