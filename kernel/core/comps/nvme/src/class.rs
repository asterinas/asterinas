// SPDX-License-Identifier: MPL-2.0

//! The `nvme` class for controllers attached to PCI devices.

use alloc::sync::Arc;

use aster_device::{
    class::{Class, ClassDevice, ClassHandle},
    common::Attr,
    uevent::UeventVars,
};
use spin::Once;

use crate::device::block_device::NvmeController;

/// The class of NVMe controllers.
pub(crate) struct NvmeClass;

impl Class for NvmeClass {
    const NAME: &'static str = "nvme";

    type Device = NvmeController;

    fn dev_attrs(&self) -> &'static [Attr<NvmeControllerDevice>] {
        &[]
    }

    fn append_uevent_vars(
        &self,
        _dev: &NvmeControllerDevice,
        _vars: &mut UeventVars,
    ) -> aster_device::common::Result<()> {
        Ok(())
    }
}

/// The device representing a controller in the `nvme` class.
pub(crate) type NvmeControllerDevice = ClassDevice<NvmeClass>;

pub(crate) static NVME_CLASS: Once<Arc<ClassHandle<NvmeClass>>> = Once::new();
