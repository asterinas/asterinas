// SPDX-License-Identifier: MPL-2.0

//! Linux-compatible /dev/tpm0 and /dev/tpmrm0 character devices,
//! registered as devices of the `tpm` class in the device model.

use alloc::vec;
use core::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

use aster_device::{
    class::{self, Class, ClassDevice, ClassHandle},
    common::{AnyDevice, Attr, DevNum},
};
use aster_tpm::{TPM_BUFSIZE, TPM_HEADER_SIZE};
use device_id::{DeviceId, MinorId};
use ostd::sync::WaitQueue;
use spin::Once;

use crate::{
    device::{Device, DeviceType, registry::char},
    events::IoEvents,
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, StatusFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    thread::work_queue::{WorkPriority, submit_work_func},
    time::{
        Timer,
        clocks::MonotonicClock,
        timer::{Timeout, TimerGuard},
    },
};

const TPM_MINOR: u32 = 224;
const TPMRM_MINOR: u32 = 225;
const TPM_MIN_WRITE: usize = 6;
const TPM_SIZE_FIELD_OFFSET: usize = 2;
const RESPONSE_TIMEOUT_SECS: u64 = 120;
const TPM2_RC_SIZE_RESPONSE: [u8; TPM_HEADER_SIZE] = [0x80, 0x01, 0, 0, 0, 0x0a, 0, 0, 0, 0x95];
const TPM2_RC_COMMAND_CODE_RESPONSE: [u8; TPM_HEADER_SIZE] =
    [0x80, 0x01, 0, 0, 0, 0x0a, 0, 0x0b, 0x01, 0x43];

static TPM0_OPEN: AtomicBool = AtomicBool::new(false);

struct WorkDrain {
    in_flight: AtomicUsize,
    wait: WaitQueue,
}

impl WorkDrain {
    fn new() -> Self {
        Self {
            in_flight: AtomicUsize::new(0),
            wait: WaitQueue::new(),
        }
    }

    fn begin(&self) {
        self.in_flight.fetch_add(1, Ordering::AcqRel);
    }

    fn finish(&self) {
        let old = self.in_flight.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(old > 0);
        if old == 1 {
            self.wait.wake_all();
        }
    }

    // The caller must prevent any new begin() operations before draining.
    fn drain(&self) {
        self.wait
            .wait_until(|| (self.in_flight.load(Ordering::Acquire) == 0).then_some(()));
    }
}

struct FileState {
    response: Vec<u8>,
    offset: usize,
    response_read: bool,
    command_enqueued: bool,
    pending_cmd: Option<Vec<u8>>,
    async_error: Option<Errno>,
}

impl FileState {
    fn new() -> Self {
        Self {
            response: Vec::new(),
            offset: 0,
            response_read: true,
            command_enqueued: false,
            pending_cmd: None,
            async_error: None,
        }
    }

    fn busy(&self) -> bool {
        self.command_enqueued
            || (!self.response_read
                && (self.offset < self.response.len() || self.async_error.is_some()))
    }

    fn has_response(&self) -> bool {
        self.async_error.is_some() || self.offset < self.response.len()
    }

    fn events(&self) -> IoEvents {
        if self.has_response() {
            IoEvents::IN
        } else {
            IoEvents::OUT
        }
    }

    fn prepare(&mut self) {
        self.response.clear();
        self.offset = 0;
        self.response_read = false;
        self.async_error = None;
        self.command_enqueued = true;
    }

    fn complete(&mut self, result: core::result::Result<Vec<u8>, Errno>) {
        self.command_enqueued = false;
        self.pending_cmd = None;
        self.response.clear();
        self.offset = 0;
        self.async_error = None;
        match result {
            Ok(response) => {
                self.response = response;
                self.response_read = false;
            }
            Err(errno) => {
                self.async_error = Some(errno);
                self.response_read = false;
            }
        }
    }

    fn read(&mut self, writer: &mut VmWriter) -> Result<usize> {
        if let Some(errno) = self.async_error.take() {
            self.response_read = true;
            return Err(Error::with_message(
                errno,
                "asynchronous TPM command failed",
            ));
        }
        if self.offset >= self.response.len() {
            return Ok(0);
        }
        let copied = match writer.write_fallible(&mut VmReader::from(&self.response[self.offset..]))
        {
            Ok(copied) => copied,
            Err(error) => {
                // Linux discards the complete response after copy_to_user() fails.
                self.response.clear();
                self.offset = 0;
                self.response_read = true;
                return Err(error.0.into());
            }
        };
        self.offset += copied;
        if copied > 0 {
            self.response_read = true;
        }
        if self.offset == self.response.len() || copied == 0 {
            self.response.clear();
            self.offset = 0;
        }
        Ok(copied)
    }
}

fn validate_command(command: &[u8]) -> Result<()> {
    if command.len() < TPM_MIN_WRITE {
        return_errno_with_message!(Errno::EINVAL, "invalid TPM command length");
    }
    let declared = u32::from_be_bytes(
        command[TPM_SIZE_FIELD_OFFSET..TPM_SIZE_FIELD_OFFSET + 4]
            .try_into()
            .unwrap(),
    ) as usize;
    if command.len() < declared {
        return_errno_with_message!(Errno::EINVAL, "command size mismatch with header");
    }
    Ok(())
}

enum Backend {
    Raw,
    ResourceManaged {
        space: aster_tpm::Space,
        ctx_buf: Vec<u8>,
        ses_buf: Vec<u8>,
    },
}

impl Backend {
    /// Takes the raw device's single-open hold, or fails with `EBUSY`.
    fn open_raw() -> Result<Self> {
        if TPM0_OPEN.swap(true, Ordering::AcqRel) {
            return_errno_with_message!(Errno::EBUSY, "/dev/tpm0 is already open");
        }
        Ok(Self::Raw)
    }

    fn open_rm() -> Self {
        Self::ResourceManaged {
            space: aster_tpm::Space::new(),
            ctx_buf: vec![0u8; aster_tpm::SPACE_BUF],
            ses_buf: vec![0u8; aster_tpm::SPACE_BUF],
        }
    }

    fn execute(
        &mut self,
        device: &aster_tpm::TpmDevice,
        mut command: Vec<u8>,
    ) -> core::result::Result<Vec<u8>, Errno> {
        if matches!(self, Self::Raw) && command.len() < TPM_HEADER_SIZE {
            return Ok(TPM2_RC_SIZE_RESPONSE.to_vec());
        }

        // Keep room for the fixed header if the chip under-reports its capacity.
        let response_len = core::cmp::max(device.limits().max_response as usize, TPM_HEADER_SIZE);
        let mut response = vec![0u8; response_len];
        let len = match self {
            Self::Raw => device
                .exec(&command, &mut response)
                .map_err(|_| Errno::EIO)?,
            Self::ResourceManaged {
                space,
                ctx_buf,
                ses_buf,
            } => {
                let command_len = command.len();
                match device.transmit_space(
                    space,
                    ctx_buf,
                    ses_buf,
                    &mut command,
                    command_len,
                    &mut response,
                ) {
                    Ok(len) => len,
                    Err(aster_tpm::XmitErr::Unsupported) => {
                        response[..TPM_HEADER_SIZE].copy_from_slice(&TPM2_RC_COMMAND_CODE_RESPONSE);
                        TPM_HEADER_SIZE
                    }
                    Err(aster_tpm::XmitErr::Malformed | aster_tpm::XmitErr::BadHandle) => {
                        return Err(Errno::EINVAL);
                    }
                    Err(
                        aster_tpm::XmitErr::NoSlots
                        | aster_tpm::XmitErr::Io(aster_tpm::IoErr::NoSpace),
                    ) => {
                        return Err(Errno::ENOMEM);
                    }
                    Err(aster_tpm::XmitErr::Io(_)) => return Err(Errno::EIO),
                }
            }
        };
        response.truncate(len);
        Ok(response)
    }

    fn close(&mut self, device: &aster_tpm::TpmDevice) {
        match self {
            Self::Raw => TPM0_OPEN.store(false, Ordering::Release),
            Self::ResourceManaged { space, .. } => device.close_space(space),
        }
    }
}

struct TpmFileShared {
    // File operations and asynchronous command completion lock the backend
    // before `state`. Timeout work takes `state` only, so it can finish
    // while a file operation drains it.
    backend: Mutex<Backend>,
    device: &'static aster_tpm::TpmDevice,
    state: Mutex<FileState>,
    pollee: Pollee,
    timer: Arc<Timer>,
    async_work: WorkDrain,
    timeout_work: WorkDrain,
}

impl TpmFileShared {
    fn new(device: &'static aster_tpm::TpmDevice, backend: Backend) -> Arc<Self> {
        Arc::new_cyclic(|weak: &Weak<Self>| {
            let weak = weak.clone();
            let timer = MonotonicClock::timer_manager().create_timer(move |_guard: TimerGuard| {
                if let Some(shared) = weak.upgrade() {
                    shared.timeout_work.begin();
                    let work_shared = shared.clone();
                    submit_work_func(
                        move || {
                            work_shared.expire_response();
                            work_shared.timeout_work.finish();
                        },
                        WorkPriority::Normal,
                    );
                }
            });
            Self {
                backend: Mutex::new(backend),
                device,
                state: Mutex::new(FileState::new()),
                pollee: Pollee::new(),
                timer,
                async_work: WorkDrain::new(),
                timeout_work: WorkDrain::new(),
            }
        })
    }

    fn drain_response_timeout(&self) {
        self.timer.lock().cancel();
        self.timeout_work.drain();
    }

    fn arm_timeout(&self) {
        if self.state.lock().has_response() {
            self.timer
                .lock()
                .set_timeout(Timeout::After(Duration::from_secs(RESPONSE_TIMEOUT_SECS)));
        }
    }

    fn expire_response(&self) {
        let mut state = self.state.lock();
        if state.has_response() {
            state.response.clear();
            state.offset = 0;
            state.async_error = None;
            state.response_read = true;
        }
        drop(state);
        self.pollee.notify(IoEvents::OUT);
    }
    /// With `report`, the command's error is returned for the synchronous
    /// writer to answer with; otherwise it is left for the next `read`.
    fn finish_command(
        &self,
        result: core::result::Result<Vec<u8>, Errno>,
        report: bool,
    ) -> Option<Errno> {
        let error = {
            let mut state = self.state.lock();
            state.complete(result);
            if report {
                state.async_error.take()
            } else {
                None
            }
        };
        self.arm_timeout();
        self.pollee
            .notify(IoEvents::IN | IoEvents::OUT | IoEvents::ERR);
        error
    }

    fn process_pending(&self) {
        let mut backend = self.backend.lock();
        let command = self.state.lock().pending_cmd.take();
        let Some(command) = command else { return };
        // Release `state` so `poll`, which takes only `state`, stays responsive.
        let result = backend.execute(self.device, command);
        self.finish_command(result, false);
    }
}

struct TpmFile {
    shared: Arc<TpmFileShared>,
}

impl Drop for TpmFile {
    fn drop(&mut self) {
        self.shared.async_work.drain();
        self.shared.drain_response_timeout();
        self.shared.backend.lock().close(self.shared.device);
    }
}

/// The `tpm` class.
struct TpmClass;

impl Class for TpmClass {
    const NAME: &'static str = "tpm";
    type Device = TpmKind;

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &[]
    }
}

/// Which of the TPM interfaces a class device represents.
#[derive(Clone, Copy)]
enum TpmKind {
    Raw,
    Rm,
}

impl TpmKind {
    fn name(self) -> &'static str {
        match self {
            Self::Raw => "tpm0",
            Self::Rm => "tpmrm0",
        }
    }

    fn minor(self) -> u32 {
        match self {
            Self::Raw => TPM_MINOR,
            Self::Rm => TPMRM_MINOR,
        }
    }
}

/// A TPM device, as seen by the char registry.
type TpmNode = ClassDevice<TpmClass>;

impl Device for TpmNode {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.devnum()
            .expect("TPM devices always have a device number")
            .id()
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        // The device model creates the node when the device is added.
        None
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        let device = aster_tpm::device()
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "no TPM device is available"))?;
        let backend = match self.payload() {
            TpmKind::Raw => Backend::open_raw()?,
            TpmKind::Rm => Backend::open_rm(),
        };
        Ok(Box::new(TpmFile {
            shared: TpmFileShared::new(device, backend),
        }))
    }
}

impl Pollable for TpmFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.shared
            .pollee
            .poll_with(mask, poller, || self.shared.state.lock().events())
    }
}

impl FileOps for TpmFile {
    fn read_at(&self, _offset: usize, writer: &mut VmWriter, _flags: StatusFlags) -> Result<usize> {
        let _backend = self.shared.backend.lock();
        let (result, drained) = {
            let mut state = self.shared.state.lock();
            let result = state.read(writer);
            let drained = !state.has_response();
            (result, drained)
        };
        if drained {
            self.shared.drain_response_timeout();
        }
        self.shared.pollee.invalidate();
        result
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader, flags: StatusFlags) -> Result<usize> {
        let command_len = reader.remain();
        if command_len > TPM_BUFSIZE {
            return_errno_with_message!(Errno::E2BIG, "TPM command too large");
        }

        let mut backend = self.shared.backend.lock();
        let command = {
            let state = self.shared.state.lock();
            if state.busy() {
                let message = match *backend {
                    Backend::Raw => "TPM file is busy",
                    Backend::ResourceManaged { .. } => "TPM resource-manager file is busy",
                };
                return_errno_with_message!(Errno::EBUSY, message);
            }
            let mut command = vec![0u8; command_len];
            reader.read_fallible(&mut VmWriter::from(&mut command[..]))?;
            validate_command(&command)?;
            command
        };

        // A partial read permits a new write. Retire any timeout belonging to
        // that old response before starting the accepted command lifecycle.
        self.shared.drain_response_timeout();

        let mut state = self.shared.state.lock();
        state.prepare();
        if flags.contains(StatusFlags::O_NONBLOCK) {
            state.pending_cmd = Some(command);
            drop(state);
            self.shared.async_work.begin();
            let shared = self.shared.clone();
            submit_work_func(
                move || {
                    shared.process_pending();
                    shared.async_work.finish();
                },
                WorkPriority::Normal,
            );
            self.shared.pollee.notify(IoEvents::OUT);
            return Ok(command_len);
        }
        drop(state);
        let result = backend.execute(self.shared.device, command);
        if let Some(errno) = self.shared.finish_command(result, true) {
            let message = match *backend {
                Backend::Raw => "TPM command failed",
                Backend::ResourceManaged { .. } => "TPM resource-manager command failed",
            };
            return Err(Error::with_message(errno, message));
        }
        Ok(command_len)
    }
}

impl PerOpenFileOps for TpmFile {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the TPM device is not seekable");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}

static TPM_CLASS: Once<Arc<ClassHandle<TpmClass>>> = Once::new();

/// Adds a TPM device of `kind` to the `tpm` class.
fn add_tpm_device(kind: TpmKind) -> Result<()> {
    let class = TPM_CLASS.get().unwrap();
    let major = super::MISC_MAJOR.get().unwrap().get();
    let id = DeviceId::new(major, MinorId::new(kind.minor()));
    let device = TpmNode::builder(class, kind.name(), kind)
        .devnum(DevNum::char(id))
        .build();
    aster_device::add_device(&device)?;
    // If the char registry refuses the device, undo the device-model registration.
    if let Err(error) = char::register(device.clone()) {
        let _ = aster_device::remove_device(&device);
        return Err(error);
    }
    Ok(())
}

pub(super) fn init_in_first_kthread() {
    let Some(device) = aster_tpm::device() else {
        return;
    };
    // Refuse a chip that cannot hold even the fixed response header.
    if device.limits().max_response < TPM_HEADER_SIZE as u32 {
        return;
    }

    TPM_CLASS.call_once(|| class::register(TpmClass).unwrap());

    add_tpm_device(TpmKind::Raw).unwrap();
    add_tpm_device(TpmKind::Rm).unwrap();
}
