// SPDX-License-Identifier: MPL-2.0

//! Exercises the real frontend against a memory backend, never firmware.
//!
//! Calls device and file operations directly with kernel buffers and injected
//! ioctl authorization results. This covers frontend decisions and cursor
//! updates; syscall/VFS offsets, real user-memory faults, credentials, and
//! CMOS/RTC synchronization require separate end-to-end tests.

use device_id::MajorId;

use super::*;
use crate::fs::file::{AccessMode, CreationFlags};

struct MemoryState {
    bytes: [u8; NVRAM_BYTES],
    invalid: bool,
    reads: usize,
    writes: usize,
    initializations: usize,
    repairs: usize,
}

// Models backend results and counts calls. Checksum failure is injected through
// `invalid`; this fixture does not implement the CMOS checksum algorithm.
struct MemoryBackend(SpinLock<MemoryState>);

impl MemoryBackend {
    fn new() -> Arc<Self> {
        Arc::new(Self(SpinLock::new(MemoryState {
            bytes: [0; NVRAM_BYTES],
            invalid: false,
            reads: 0,
            writes: 0,
            initializations: 0,
            repairs: 0,
        })))
    }
}

impl Backend for MemoryBackend {
    fn size(&self) -> core::result::Result<usize, NvramError> {
        Ok(NVRAM_BYTES)
    }

    fn read(&self, offset: usize, output: &mut [u8]) -> core::result::Result<usize, NvramError> {
        let mut state = self.0.lock();
        state.reads += 1;
        if state.invalid {
            return Err(NvramError::InvalidChecksum);
        }
        output.copy_from_slice(&state.bytes[offset..offset + output.len()]);
        Ok(output.len())
    }

    fn write(&self, offset: usize, input: &[u8]) -> core::result::Result<usize, NvramError> {
        let mut state = self.0.lock();
        state.writes += 1;
        if state.invalid {
            return Err(NvramError::InvalidChecksum);
        }
        state.bytes[offset..offset + input.len()].copy_from_slice(input);
        Ok(input.len())
    }

    fn initialize(&self) -> core::result::Result<(), NvramError> {
        let mut state = self.0.lock();
        state.initializations += 1;
        state.bytes.fill(0);
        state.invalid = false;
        Ok(())
    }

    fn set_checksum(&self) -> core::result::Result<(), NvramError> {
        let mut state = self.0.lock();
        state.repairs += 1;
        state.invalid = false;
        Ok(())
    }
}

fn context(mode: AccessMode, exclusive: bool) -> DeviceOpenContext {
    DeviceOpenContext::new(
        mode,
        if exclusive {
            CreationFlags::O_EXCL
        } else {
            CreationFlags::empty()
        },
    )
}

fn assert_busy(result: Result<Box<dyn PerOpenFileOps>>) {
    assert_eq!(result.err().unwrap().error(), Errno::EBUSY);
}

#[ostd::prelude::ktest]
fn nvram_frontend_contract() {
    let backend = MemoryBackend::new();
    let id = DeviceId::new(MajorId::new(10), MinorId::new(NVRAM_MINOR));
    let device = NvramDevice::new(id, backend.clone()).unwrap();
    assert_eq!(device.id().to_raw(), id.to_raw());
    assert!(device.devtmpfs_meta().is_some());
    let read = context(AccessMode::O_RDONLY, false);
    let write = context(AccessMode::O_WRONLY, false);
    let rw = context(AccessMode::O_RDWR, false);
    let exclusive = context(AccessMode::O_RDONLY, true);

    // Multiple readers and one writer may coexist. Rejected opens change no state.
    let reader1 = device.open(&read).unwrap();
    let reader2 = device.open(&read).unwrap();
    let writer: Arc<dyn PerOpenFileOps> = Arc::from(device.open(&rw).unwrap());
    assert_busy(device.open(&write));
    assert_busy(device.open(&exclusive));
    assert_eq!(device.shared.opens.lock().users, 3);
    // Model shared open-file ownership; this does not execute dup/fork/close.
    let duplicate = writer.clone();
    drop(writer);
    assert_busy(device.open(&rw));
    drop(duplicate);
    drop(device.open(&write).unwrap());
    drop(reader1);
    drop(reader2);
    let only = device.open(&exclusive).unwrap();
    assert_busy(device.open(&read));
    assert_busy(device.open(&write));
    drop(only);
    assert_eq!(device.shared.opens.lock().users, 0);

    let file = device.open(&rw).unwrap();
    assert!(file.check_seekable().is_ok());
    assert!(file.is_offset_aware());
    assert_eq!(file.seek_end().unwrap(), Some(NVRAM_BYTES));
    assert_eq!(file.poll(IoEvents::all(), None).bits(), 0x145);
    let payload = [17, 34, 51, 68];
    let mut input = VmReader::from(payload.as_slice()).to_fallible();
    assert_eq!(
        file.write_at(64, &mut input, StatusFlags::O_NONBLOCK)
            .unwrap(),
        4
    );
    assert_eq!(input.remain(), 0);
    let mut output = [0; 4];
    assert_eq!(
        file.read_at(
            64,
            &mut VmWriter::from(output.as_mut_slice()).to_fallible(),
            StatusFlags::empty()
        )
        .unwrap(),
        4
    );
    assert_eq!(output, payload);

    // Short write consumes only the bytes actually stored. Short read preserves the tail.
    let mut input = VmReader::from(payload.as_slice()).to_fallible();
    assert_eq!(
        file.write_at(112, &mut input, StatusFlags::empty())
            .unwrap(),
        2
    );
    assert_eq!(input.remain(), 2);
    let mut output = [0xaa; 4];
    assert_eq!(
        file.read_at(
            112,
            &mut VmWriter::from(output.as_mut_slice()).to_fallible(),
            StatusFlags::empty()
        )
        .unwrap(),
        2
    );
    assert_eq!(output, [17, 34, 0xaa, 0xaa]);

    backend.0.lock().invalid = true;
    let reads = backend.0.lock().reads;
    let writes = backend.0.lock().writes;
    for offset in [NVRAM_BYTES, NVRAM_BYTES + 10, usize::MAX] {
        let mut output = [0xaa; 1];
        let mut cursor = VmWriter::from(output.as_mut_slice()).to_fallible();
        assert_eq!(
            file.read_at(offset, &mut cursor, StatusFlags::empty())
                .unwrap(),
            0
        );
        assert_eq!(cursor.avail(), 1);
        let mut cursor = VmReader::from(payload.as_slice()).to_fallible();
        assert_eq!(
            file.write_at(offset, &mut cursor, StatusFlags::empty())
                .unwrap(),
            0
        );
        assert_eq!(cursor.remain(), 4);
    }
    assert_eq!(backend.0.lock().reads, reads);
    assert_eq!(backend.0.lock().writes, writes);
    let mut cursor = VmReader::from(payload.as_slice()).to_fallible();
    assert_eq!(
        file.write_at(0, &mut cursor, StatusFlags::empty())
            .unwrap_err()
            .error(),
        Errno::EIO
    );
    assert_eq!(cursor.remain(), 4);
    let mut output = [0xaa; 1];
    let mut cursor = VmWriter::from(output.as_mut_slice()).to_fallible();
    assert_eq!(
        file.read_at(0, &mut cursor, StatusFlags::empty())
            .unwrap_err()
            .error(),
        Errno::EIO
    );
    assert_eq!(cursor.avail(), 1);
    assert_eq!(
        file.read_at(
            0,
            &mut VmWriter::from(&mut [][..]).to_fallible(),
            StatusFlags::empty()
        )
        .unwrap_err()
        .error(),
        Errno::EIO
    );
    assert_eq!(
        file.write_at(
            0,
            &mut VmReader::from(&[][..]).to_fallible(),
            StatusFlags::empty()
        )
        .unwrap_err()
        .error(),
        Errno::EIO
    );
    drop(file);

    // Both known ioctls require authorization but do not require a writable open.
    // Injected closures test authorization ordering and denial propagation;
    // they do not exercise check_sys_admin or the caller's actual credentials.
    let file = device.open(&read).unwrap();
    let frontend = (file.as_ref() as &dyn Any)
        .downcast_ref::<NvramFile>()
        .unwrap();
    for command in [0x7040, 0x7041] {
        let denied = frontend.handle_ioctl(RawIoctl::new(command, usize::MAX), || {
            Err(Error::new(Errno::EACCES))
        });
        assert_eq!(denied.unwrap_err().error(), Errno::EACCES);
    }
    assert_eq!(backend.0.lock().initializations, 0);
    assert_eq!(backend.0.lock().repairs, 0);
    for command in [0xffff, 0x80007040, 0x40047041] {
        assert_eq!(
            frontend
                .handle_ioctl(RawIoctl::new(command, 0), || panic!(
                    "unknown ioctl must not authorize"
                ))
                .unwrap_err()
                .error(),
            Errno::ENOTTY
        );
    }
    assert_eq!(
        frontend
            .handle_ioctl(RawIoctl::new(0x7041, usize::MAX), || Ok(()))
            .unwrap(),
        0
    );
    assert_eq!(backend.0.lock().repairs, 1);
    assert_eq!(&backend.0.lock().bytes[64..68], &payload);
    assert_eq!(
        frontend
            .handle_ioctl(RawIoctl::new(0x7040, usize::MAX), || Ok(()))
            .unwrap(),
        0
    );
    assert_eq!(backend.0.lock().initializations, 1);
    assert_eq!(backend.0.lock().bytes, [0; NVRAM_BYTES]);
    assert_eq!(
        backend_error(NvramError::Unavailable).error(),
        Errno::ENODEV
    );
}
