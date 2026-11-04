// SPDX-License-Identifier: MPL-2.0

//! Linux-compatible /dev/tpm0 and /dev/tpmrm0 character-device bridge.

use alloc::vec;
use core::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

use device_id::{DeviceId, MinorId};
use ostd::sync::WaitQueue;

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
const TPM_BUFSIZE: usize = 4096;
const TPM_MIN_WRITE: usize = 6;
const TPM_HEADER_SIZE: usize = 10;
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
    let declared = u32::from_be_bytes([command[2], command[3], command[4], command[5]]) as usize;
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
    fn execute(
        &mut self,
        device: &aster_tpm::TpmDevice,
        mut command: Vec<u8>,
    ) -> core::result::Result<Vec<u8>, Errno> {
        if matches!(self, Self::Raw) && command.len() < TPM_HEADER_SIZE {
            return Ok(TPM2_RC_SIZE_RESPONSE.to_vec());
        }

        let mut response = vec![0u8; device.limits().max_response as usize];
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
    // File operations and asynchronous command completion take this lock
    // before state. Timeout work takes state only, so it can finish while a
    // file operation drains it.
    operation_lock: Mutex<Backend>,
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
                operation_lock: Mutex::new(backend),
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
                .set_timeout(Timeout::After(Duration::from_secs(120)));
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
    fn process_pending(&self) {
        let mut backend = self.operation_lock.lock();
        let mut state = self.state.lock();
        let command = state.pending_cmd.take();
        let Some(command) = command else { return };
        let result = if matches!(*backend, Backend::Raw) {
            drop(state);
            let result = backend.execute(self.device, command);
            state = self.state.lock();
            result
        } else {
            backend.execute(self.device, command)
        };
        state.complete(result);
        drop(state);
        self.arm_timeout();
        self.pollee
            .notify(IoEvents::IN | IoEvents::OUT | IoEvents::ERR);
    }
}
struct TpmFile {
    shared: Arc<TpmFileShared>,
}
impl Drop for TpmFile {
    fn drop(&mut self) {
        self.shared.async_work.drain();
        self.shared.drain_response_timeout();
        self.shared.operation_lock.lock().close(self.shared.device);
    }
}

#[derive(Debug)]
struct TpmDev(DeviceId);
impl TpmDev {
    fn new() -> Arc<Self> {
        let major = super::MISC_MAJOR.get().unwrap().get();
        Arc::new(Self(DeviceId::new(major, MinorId::new(TPM_MINOR))))
    }
}
impl Device for TpmDev {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }
    fn id(&self) -> DeviceId {
        self.0
    }
    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        Some(DevtmpfsNodeMeta::new("tpm0").unwrap())
    }
    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        let device = aster_tpm::device()
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "no TPM device is available"))?;
        if TPM0_OPEN.swap(true, Ordering::Acquire) {
            return_errno_with_message!(Errno::EBUSY, "/dev/tpm0 is already open");
        }
        Ok(Box::new(TpmFile {
            shared: TpmFileShared::new(device, Backend::Raw),
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
        let _op = self.shared.operation_lock.lock();
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

        let mut backend = self.shared.operation_lock.lock();
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
        let result = backend.execute(self.shared.device, command);
        state.complete(result);
        let error = state.async_error.take();
        drop(state);
        self.shared.arm_timeout();
        self.shared
            .pollee
            .notify(IoEvents::IN | IoEvents::OUT | IoEvents::ERR);
        if let Some(errno) = error {
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

#[derive(Debug)]
struct TpmRmDev(DeviceId);
impl TpmRmDev {
    fn new() -> Arc<Self> {
        let major = super::MISC_MAJOR.get().unwrap().get();
        Arc::new(Self(DeviceId::new(major, MinorId::new(TPMRM_MINOR))))
    }
}
impl Device for TpmRmDev {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }
    fn id(&self) -> DeviceId {
        self.0
    }
    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        Some(DevtmpfsNodeMeta::new("tpmrm0").unwrap())
    }
    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        let device = aster_tpm::device()
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "no TPM device is available"))?;
        Ok(Box::new(TpmFile {
            shared: TpmFileShared::new(
                device,
                Backend::ResourceManaged {
                    space: aster_tpm::Space::new(),
                    ctx_buf: vec![0u8; aster_tpm::SPACE_BUF],
                    ses_buf: vec![0u8; aster_tpm::SPACE_BUF],
                },
            ),
        }))
    }
}
pub(super) fn init_in_first_kthread() {
    if aster_tpm::device().is_some() {
        char::register(TpmDev::new()).unwrap();
        char::register(TpmRmDev::new()).unwrap();
    }
}
