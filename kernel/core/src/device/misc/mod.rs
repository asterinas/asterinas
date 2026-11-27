// SPDX-License-Identifier: MPL-2.0

//! Misc devices.
//!
//! Character device with major number 10.

use aster_device::{
    class::{self, Class, ClassDevice, ClassHandle},
    common::{AnyDevice, Attr, DevNode},
    uevent::UeventVars,
};
use device_id::{DeviceId, MajorId};
use spin::Once;

use super::{
    Device, DeviceType,
    registry::char::{self, MajorIdOwner},
};
use crate::{
    fs::{devtmpfs::DevtmpfsNodeMeta, file::PerOpenFileOps},
    prelude::*,
};

mod hwrng;
#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
pub(crate) mod tdxguest;

static MISC_CLASS: Once<Arc<ClassHandle<MiscClass>>> = Once::new();
static MISC_MAJOR: Once<MajorIdOwner> = Once::new();

pub(super) fn init_in_first_kthread() {
    MISC_MAJOR.call_once(|| char::acquire_major(MajorId::new(10)).unwrap());
    MISC_CLASS.call_once(|| class::register(MiscClass).unwrap());

    hwrng::init_in_first_kthread().unwrap();

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        tdxguest::init().unwrap();
    });
}

struct MiscClass;

impl Class for MiscClass {
    const NAME: &'static str = "misc";

    type Device = Box<dyn AnyMiscDevice>;

    fn devnode(&self, dev: &ClassDevice<Self>) -> Option<DevNode> {
        dev.payload().devnode()
    }

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &[]
    }

    fn append_uevent_vars(
        &self,
        _dev: &ClassDevice<Self>,
        _vars: &mut UeventVars,
    ) -> aster_device::common::Result<()> {
        Ok(())
    }
}

trait AnyMiscDevice: Send + Sync + 'static {
    /// Opens the device node and returns the file operations for the resulting handle.
    ///
    /// Successfully registering a misc class device **always** creates a character device node under `/dev`.
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/misc.c#L251>.
    fn open(&self) -> Result<Box<dyn PerOpenFileOps>>;

    /// Overrides the device node's path or permissions.
    /// Returning `None` uses the device name and the default devtmpfs permissions.
    fn devnode(&self) -> Option<DevNode> {
        None
    }
}

impl Device for ClassDevice<MiscClass> {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.devnum()
            .expect("misc devices have a device number")
            .id()
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        // The device model creates the devtmpfs node.
        None
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        self.payload().open()
    }
}
