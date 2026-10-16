// SPDX-License-Identifier: MPL-2.0

//! The x86 `/dev/nvram` character frontend.
//!
//! Provides read/write operations, open admission, basic seeking, and the
//! initialization/checksum-repair ioctls based on Linux `drivers/char/nvram.c`.
//! Known VFS compatibility gaps remain for `SEEK_DATA`/`SEEK_HOLE` and empty
//! vectored I/O: the seek interface lacks those modes, and empty vectors can
//! bypass access-mode checks or trigger an extra backend checksum check.
//!
//! CMOS ownership, checksum transactions and IRQ exclusion belong to `aster-time`.

use aster_time::NvramError;
use device_id::{DeviceId, MinorId};

use crate::{
    device::{Device, DeviceOpenContext, DeviceType, registry::char},
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, StatusFlags},
        vfs::{inode::FileOps, path::Path},
    },
    prelude::*,
    process::{
        UserNamespace,
        credentials::capabilities::CapSet,
        posix_thread::AsPosixThread,
        signal::{PollHandle, Pollable},
    },
    security::lsm::hooks as lsm_hooks,
    util::ioctl::RawIoctl,
};

// References:
// <https://elixir.bootlin.com/linux/v6.18/source/include/linux/miscdevice.h>.
// <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/nvram.h>.
const NVRAM_MINOR: u32 = 144;
const NVRAM_BYTES: usize = 128 - 14;

#[cfg(ktest)]
mod test;

mod ioctl_defs {
    use crate::{ioc, util::ioctl::NoData};

    // NVRAM_INIT clears the region and initializes its checksum.
    pub(super) type Initialize = ioc!(NVRAM_INIT, b'p', 0x40, NoData);
    // NVRAM_SETCKS recomputes the checksum while preserving the other data.
    pub(super) type SetChecksum = ioc!(NVRAM_SETCKS, b'p', 0x41, NoData);
}

/// Driver-local boundary allowing the frontend to be tested without CMOS writes.
trait Backend: Send + Sync {
    fn size(&self) -> core::result::Result<usize, NvramError>;
    fn read(&self, offset: usize, output: &mut [u8]) -> core::result::Result<usize, NvramError>;
    fn write(&self, offset: usize, input: &[u8]) -> core::result::Result<usize, NvramError>;
    fn initialize(&self) -> core::result::Result<(), NvramError>;
    fn set_checksum(&self) -> core::result::Result<(), NvramError>;
}

struct CmosBackend;

impl Backend for CmosBackend {
    fn size(&self) -> core::result::Result<usize, NvramError> {
        Ok(aster_time::nvram_backend()?.size())
    }

    fn read(&self, offset: usize, output: &mut [u8]) -> core::result::Result<usize, NvramError> {
        aster_time::nvram_backend()?.read(offset, output)
    }

    fn write(&self, offset: usize, input: &[u8]) -> core::result::Result<usize, NvramError> {
        aster_time::nvram_backend()?.write(offset, input)
    }

    fn initialize(&self) -> core::result::Result<(), NvramError> {
        aster_time::nvram_backend()?.initialize();
        Ok(())
    }

    fn set_checksum(&self) -> core::result::Result<(), NvramError> {
        aster_time::nvram_backend()?.set_checksum();
        Ok(())
    }
}

#[derive(Default)]
struct OpenState {
    users: usize,
    writer: bool,
    exclusive: bool,
}

struct Shared {
    backend: Arc<dyn Backend>,
    size: usize,
    // Admission never holds this lock while accessing CMOS or user memory.
    opens: SpinLock<OpenState>,
}

struct NvramDevice {
    id: DeviceId,
    shared: Arc<Shared>,
}

impl NvramDevice {
    fn new(id: DeviceId, backend: Arc<dyn Backend>) -> Result<Self> {
        let size = backend.size().map_err(backend_error)?;
        if size != NVRAM_BYTES {
            return_errno_with_message!(Errno::ENODEV, "unsupported x86 NVRAM layout");
        }
        Ok(Self {
            id,
            shared: Arc::new(Shared {
                backend,
                size,
                opens: SpinLock::new(OpenState::default()),
            }),
        })
    }
}

impl Device for NvramDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        Some(DevtmpfsNodeMeta::new("nvram").unwrap())
    }

    fn open(&self, context: &DeviceOpenContext) -> Result<Box<dyn PerOpenFileOps>> {
        {
            let mut state = self.shared.opens.lock();
            if state.exclusive
                || (context.is_exclusive() && state.users != 0)
                || (context.is_writable() && state.writer)
            {
                return_errno_with_message!(Errno::EBUSY, "NVRAM is already in use");
            }
            // Each user owns a separately allocated open file description.
            // Their number cannot reach usize::MAX in the address space.
            state.users += 1;
            state.writer |= context.is_writable();
            state.exclusive = context.is_exclusive();
        }
        Ok(Box::new(NvramFile {
            shared: self.shared.clone(),
            context: *context,
        }))
    }
}

struct NvramFile {
    shared: Arc<Shared>,
    context: DeviceOpenContext,
}

impl Drop for NvramFile {
    fn drop(&mut self) {
        // Owned by InodeHandle: dup/fork share the description, so this runs
        // only when its final reference disappears.
        let mut state = self.shared.opens.lock();
        debug_assert!(state.users > 0);
        state.users -= 1;
        if self.context.is_writable() {
            state.writer = false;
        }
        if self.context.is_exclusive() {
            state.exclusive = false;
        }
    }
}

impl FileOps for NvramFile {
    fn read_at(&self, offset: usize, writer: &mut VmWriter, _flags: StatusFlags) -> Result<usize> {
        if offset >= self.shared.size {
            return Ok(0);
        }
        let count = writer.avail().min(self.shared.size - offset).min(PAGE_SIZE);
        let mut buffer = [0; NVRAM_BYTES];
        let count = self
            .shared
            .backend
            .read(offset, &mut buffer[..count])
            .map_err(backend_error)?;
        // Copy after the backend releases its IRQ-disabled hardware lock.
        // Return the fault, not a partial count: InodeHandle then leaves the
        // file offset unchanged, as Linux does after copy_to_user failure.
        let mut output = writer.clone_exclusive();
        let copied = output
            .write_fallible(&mut VmReader::from(&buffer[..count]))
            .map_err(|(error, _copied)| error)?;
        writer.skip(copied);
        Ok(copied)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader, _flags: StatusFlags) -> Result<usize> {
        if offset >= self.shared.size {
            return Ok(0);
        }
        let count = reader
            .remain()
            .min(self.shared.size - offset)
            .min(PAGE_SIZE);
        let mut buffer = [0; NVRAM_BYTES];
        // Linux memdup_user occurs before the backend checksum check.
        let mut input = reader.clone();
        input
            .read_fallible(&mut VmWriter::from(&mut buffer[..count]))
            .map_err(|(error, _copied)| error)?;
        let written = self
            .shared
            .backend
            .write(offset, &buffer[..count])
            .map_err(backend_error)?;
        reader.skip(written);
        Ok(written)
    }
}

impl Pollable for NvramFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        // Linux uses the default mask because nvram has no .poll callback.
        mask & (IoEvents::IN | IoEvents::OUT | IoEvents::RDNORM | IoEvents::WRNORM)
    }
}

impl PerOpenFileOps for NvramFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn seek_end(&self) -> Result<Option<usize>> {
        Ok(Some(self.shared.size))
    }

    fn ioctl(&self, _path: &Path, command: RawIoctl) -> Result<i32> {
        self.handle_ioctl(command, check_sys_admin)
    }
}

impl NvramFile {
    fn handle_ioctl(
        &self,
        command: RawIoctl,
        authorize_fn: impl FnOnce() -> Result<()>,
    ) -> Result<i32> {
        use ioctl_defs::*;

        dispatch_ioctl!(match command {
            _cmd @ Initialize => {
                authorize_fn()?;
                self.shared.backend.initialize().map_err(backend_error)?;
                Ok(0)
            }
            _cmd @ SetChecksum => {
                authorize_fn()?;
                self.shared.backend.set_checksum().map_err(backend_error)?;
                Ok(0)
            }
            _ => return_errno_with_message!(Errno::ENOTTY, "unknown NVRAM ioctl"),
        })
    }
}

fn check_sys_admin() -> Result<()> {
    let thread = current_thread!();
    let posix = thread.as_posix_thread().unwrap();
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix,
        CapSet::SYS_ADMIN,
    ))
    .map_err(|_| Error::with_message(Errno::EACCES, "NVRAM ioctl requires CAP_SYS_ADMIN"))
}

fn backend_error(error: NvramError) -> Error {
    match error {
        NvramError::InvalidChecksum => Error::with_message(Errno::EIO, "invalid NVRAM checksum"),
        NvramError::Unavailable => Error::with_message(Errno::ENODEV, "NVRAM is unavailable"),
    }
}

pub(super) fn init_in_first_kthread() {
    let id = DeviceId::new(
        super::MISC_MAJOR.get().unwrap().get(),
        MinorId::new(NVRAM_MINOR),
    );
    let device = match NvramDevice::new(id, Arc::new(CmosBackend)) {
        Ok(device) => device,
        Err(error) => {
            debug!("not registering /dev/nvram: {:?}", error);
            return;
        }
    };
    if let Err(error) = char::register(Arc::new(device)) {
        warn!("failed to register /dev/nvram: {:?}", error);
    }
}
