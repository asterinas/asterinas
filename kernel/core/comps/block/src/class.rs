// SPDX-License-Identifier: MPL-2.0

//! The block class for whole disks and their partitions.

use alloc::{boxed::Box, sync::Arc};

use aster_device::{
    class::{Class, ClassDevice, ClassHandle},
    common::{Attr, DeviceType},
    uevent::UeventVars,
};
use spin::Once;

use crate::AnyBlockDevice;

/// The class of block devices.
pub struct BlockClass;

/// A whole disk or partition registered in the device model.
pub type BlockDevice = ClassDevice<BlockClass>;

/// The registered block class.
pub static BLOCK_CLASS: Once<Arc<ClassHandle<BlockClass>>> = Once::new();

/// The device type for whole disks.
pub static DISK_TYPE: DeviceType<BlockDevice> = DeviceType::named("disk");

impl Class for BlockClass {
    const NAME: &'static str = "block";

    type Device = Box<dyn AnyBlockDevice>;

    fn dev_attrs(&self) -> &'static [Attr<BlockDevice>] {
        const ATTRS: &[Attr<BlockDevice>] = &[Attr::ro("size", |dev, w| {
            writeln!(w, "{}", dev.metadata().nr_sectors)?;
            Ok(())
        })];
        ATTRS
    }

    fn append_uevent_vars(
        &self,
        _dev: &BlockDevice,
        _vars: &mut UeventVars,
    ) -> aster_device::common::Result<()> {
        Ok(())
    }
}
