// SPDX-License-Identifier: MPL-2.0

//! A vendor-matched bus whose driver creates a child block device.
//!
//! `ToyBus` devices carry vendor and model IDs.
//! `ToyDiskDriver` matches the vendor and creates a `ToyDisk` in the `ToyBlock` class.
//! `CountingObserver` counts the class's device notifications.

use alloc::{string::ToString, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_systree::SysObj;
use device_id::{DeviceId, MajorId, MinorId};

use super::utils;
use crate::{
    bus::{self, Bus, BusDevice, BusHandle, Driver},
    class::{self, Class, ClassDevice, ClassHandle, ClassObserver},
    common::{Attr, DevNode, DevNum, DeviceType, Result},
};

/// Returns the shared toy class and bus, registering them on first use.
pub(super) fn register_toy_subsystems() -> ToySubsystems {
    static TOY_SUBSYSTEMS: spin::Once<ToySubsystems> = spin::Once::new();

    crate::init_for_ktest();
    TOY_SUBSYSTEMS
        .call_once(|| {
            let bus = bus::register(ToyBus).unwrap();
            let class = class::register(ToyBlock).unwrap();
            (class, bus)
        })
        .clone()
}

/// A toy bus: devices carry a vendor and a model id, drivers accept a vendor.
pub(super) struct ToyBus;

pub(super) struct ToyDev {
    pub(super) vendor: u32,
    pub(super) model: u32,
}

pub(super) struct ToyMatch {
    pub(super) vendor: u32,
}

impl Bus for ToyBus {
    const NAME: &'static str = "toy";
    type Device = ToyDev;
    type MatchData = ToyMatch;

    fn matches(&self, dev: &ToyDev, data: &ToyMatch) -> bool {
        dev.vendor == data.vendor
    }

    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>] {
        TOY_DEV_ATTRS
    }
}

pub(super) static DISK_TYPE: DeviceType<BusDevice<ToyBus>> = DeviceType::named("disk");

/// A driver that creates a toy disk on probe.
pub(super) struct ToyDiskDriver {
    pub(super) match_data: ToyMatch,
    pub(super) class: Arc<ClassHandle<ToyBlock>>,
}

impl Driver<ToyBus> for ToyDiskDriver {
    fn name(&self) -> &str {
        "toy_disk"
    }

    fn match_data(&self) -> &ToyMatch {
        &self.match_data
    }

    fn on_probe(&self, dev: &Arc<BusDevice<ToyBus>>) -> Result<()> {
        let name = alloc::format!("td{}", dev.model);
        let disk = ClassDevice::builder(&self.class, name, ToyDisk { sectors: 8 })
            .parent(dev.clone())
            .devnum(DevNum::block(DeviceId::new(
                MajorId::new(200),
                MinorId::new(dev.model),
            )))
            .build();
        crate::add_device(&disk)?;
        Ok(())
    }

    fn on_release(&self, dev: &Arc<BusDevice<ToyBus>>) {
        let path = dev.path().to_string();
        assert!(utils::lookup(&alloc::format!("{path}/driver")).is_none());
        assert!(
            !utils::attr_names(&path)
                .iter()
                .any(|name| name == "bound_by")
        );

        let name = alloc::format!("td{}", dev.model);
        let disk = self.class.find_device(&name).unwrap();
        crate::remove_device(&disk).unwrap();
    }

    fn dev_attrs(&self) -> &'static [Attr<BusDevice<ToyBus>>] {
        BOUND_BY
    }
}

/// A toy block class.
pub(super) struct ToyBlock;

pub(super) struct ToyDisk {
    pub(super) sectors: u64,
}

impl Class for ToyBlock {
    const NAME: &'static str = "toyblk";
    type Device = ToyDisk;

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        TOY_BLOCK_ATTRS
    }

    fn devnode(&self, _dev: &ClassDevice<Self>) -> Option<DevNode> {
        Some(DevNode {
            path: None,
            mode: Some(0o660),
        })
    }
}

pub(super) struct CountingObserver {
    pub(super) added: AtomicUsize,
    pub(super) removed: AtomicUsize,
}

impl ClassObserver<ToyBlock> for CountingObserver {
    fn on_device_added(&self, _dev: &Arc<ClassDevice<ToyBlock>>) {
        self.added.fetch_add(1, Ordering::Relaxed);
    }

    fn on_device_removed(&self, _dev: &Arc<ClassDevice<ToyBlock>>) {
        self.removed.fetch_add(1, Ordering::Relaxed);
    }
}

type ToySubsystems = (Arc<ClassHandle<ToyBlock>>, Arc<BusHandle<ToyBus>>);

const TOY_DEV_ATTRS: &[Attr<BusDevice<ToyBus>>] = &[
    Attr::ro("vendor", |dev, w| {
        writeln!(w, "{:#06x}", dev.vendor)?;
        Ok(())
    }),
    Attr::ro("model", |dev, w| {
        writeln!(w, "{:#06x}", dev.model)?;
        Ok(())
    }),
];

const BOUND_BY: &[Attr<BusDevice<ToyBus>>] = &[Attr::ro("bound_by", |_dev, w| {
    writeln!(w, "toy_disk")?;
    Ok(())
})];

const TOY_BLOCK_ATTRS: &[Attr<ClassDevice<ToyBlock>>] = &[Attr::ro("size", |dev, w| {
    writeln!(w, "{}", dev.sectors)?;
    Ok(())
})];
