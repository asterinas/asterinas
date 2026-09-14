// SPDX-License-Identifier: MPL-2.0

//! Interfaces and core adapters for simple misc character devices.
//!
//! The interface in this module is intentionally narrower than the kernel's
//! internal device and VFS abstractions. It supports offset-unaware misc
//! devices that use the VFS default polling behavior and do not support
//! directory operations, memory mapping, or synchronization.

use alloc::{boxed::Box, sync::Arc};

use aster_util::safe_ptr::SafePtr;
use device_id::{DeviceId, MajorId, MinorId};
use ostd::mm::{VmReader, VmWriter};
use ostd_pod::Pod;
use spin::Once;

use super::{
    Device, DeviceType,
    registry::char::{self, MajorIdOwner, acquire_major},
};
pub use crate::{
    context::CurrentUserSpace,
    error::{Errno, Error},
    util::ioctl::RawIoctl,
};
use crate::{
    events::IoEvents,
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, StatusFlags},
        vfs::{inode::FileOps, path::Path},
    },
    process::signal::{PollHandle, Pollable},
    util::ioctl::{InOutData, Ioctl},
};

mod hwrng;
#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
pub(crate) mod tdxguest;

/// The result type used by misc device operations.
pub type Result<T> = core::result::Result<T, Error>;

/// A modern ioctl command whose argument is an input/output object.
///
/// `MAGIC` and `NR` are the type and sequence-number fields in the Linux ioctl
/// encoding. The command is accepted only when its direction and argument size
/// also match `T`.
pub struct InOutIoctl<const MAGIC: u8, const NR: u8, T: Pod>(Ioctl<MAGIC, NR, true, InOutData<T>>);

impl<const MAGIC: u8, const NR: u8, T: Pod> InOutIoctl<MAGIC, NR, T> {
    /// Tries to interpret a raw request as this ioctl command.
    pub fn try_from_raw(request: RawIoctl) -> Option<Self> {
        Ioctl::try_from_raw(request).map(Self)
    }

    /// Provides field-level access to the ioctl argument in userspace.
    pub fn with_data_ptr<F, R>(&self, f: F) -> Result<R>
    where
        F: for<'a> FnOnce(SafePtr<T, CurrentUserSpace<'a>>) -> Result<R>,
    {
        self.0.with_data_ptr(f)
    }
}

/// A simple misc character device.
///
/// Implementations create one operation object for each open file description.
pub trait MiscDevice: Send + Sync + 'static {
    /// Opens the device.
    fn open(&self) -> Result<Box<dyn MiscDeviceFile>>;
}

/// Operations on one open file description of a simple misc device.
///
/// Files exposed through this interface are offset-unaware and use the VFS
/// default polling behavior, which reports requested read and write events as
/// ready. Operations outside this interface retain the defaults of the
/// kernel's internal per-open file abstraction.
pub trait MiscDeviceFile: Send + Sync + 'static {
    /// Checks whether seeking is supported.
    ///
    /// A successful result makes seeking a no-op because misc files exposed by
    /// this interface are offset-unaware.
    fn check_seekable(&self) -> Result<()>;

    /// Reads bytes into `writer`.
    fn read(&self, writer: &mut VmWriter, is_nonblocking: bool) -> Result<usize>;

    /// Writes bytes from `reader`.
    fn write(&self, reader: &mut VmReader, is_nonblocking: bool) -> Result<usize>;

    /// Handles an ioctl request.
    fn ioctl(&self, _request: RawIoctl) -> Result<i32> {
        Err(Error::with_message(Errno::ENOTTY, "ioctl is not supported"))
    }
}

/// Registers a simple misc character device.
///
/// The device is assigned misc major 10 and the supplied minor number. The
/// node is created in devtmpfs with the default owner read/write permissions.
/// Misc initialization and the devtmpfs worker must be ready before this
/// function is called.
pub fn register_misc_device(
    minor: u32,
    node_path: &'static str,
    device: Arc<dyn MiscDevice>,
) -> Result<()> {
    let major = MISC_MAJOR
        .get()
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "misc devices are not initialized"))?
        .get();
    let metadata = DevtmpfsNodeMeta::new(node_path)
        .map_err(|_| Error::with_message(Errno::EINVAL, "the devtmpfs path is invalid"))?;
    let minor = MinorId::try_from(minor).map_err(|msg| Error::with_message(Errno::EINVAL, msg))?;
    let device = MiscDeviceAdapter {
        id: DeviceId::new(major, minor),
        metadata,
        device,
    };
    char::register(Arc::new(device))
}

struct MiscDeviceAdapter {
    id: DeviceId,
    metadata: DevtmpfsNodeMeta,
    device: Arc<dyn MiscDevice>,
}

impl Device for MiscDeviceAdapter {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        Some(self.metadata.clone())
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        let file = self.device.open()?;
        Ok(Box::new(MiscDeviceFileAdapter(file)))
    }
}

struct MiscDeviceFileAdapter(Box<dyn MiscDeviceFile>);

impl Pollable for MiscDeviceFileAdapter {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        mask & (IoEvents::IN | IoEvents::OUT)
    }
}

impl FileOps for MiscDeviceFileAdapter {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0
            .read(writer, status_flags.contains(StatusFlags::O_NONBLOCK))
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0
            .write(reader, status_flags.contains(StatusFlags::O_NONBLOCK))
    }
}

impl PerOpenFileOps for MiscDeviceFileAdapter {
    fn check_seekable(&self) -> Result<()> {
        self.0.check_seekable()
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, _path: &Path, request: RawIoctl) -> Result<i32> {
        self.0.ioctl(request)
    }
}

static MISC_MAJOR: Once<MajorIdOwner> = Once::new();

pub(super) fn init_in_first_kthread() {
    MISC_MAJOR.call_once(|| acquire_major(MajorId::new(10)).unwrap());

    hwrng::init_in_first_kthread();

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        tdxguest::init().unwrap();
    });
}
