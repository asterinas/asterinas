// SPDX-License-Identifier: MPL-2.0

//! Ownership and copy boundaries for Linux DM ioctl parameters.
//!
//! The fixed header is captured once. Secure buffers are wiped on every exit;
//! dispatch runs only after copyin and user clearing, and only success copies out.
//! User-memory access and command execution are supplied by the control file.

use core::ops::{Deref, DerefMut};

use super::abi::{
    DM_DEV_CREATE_CMD, DM_DEV_REMOVE_CMD, DM_DEV_STATUS_CMD, DM_IOCTL_FIXED_PREFIX_SIZE,
    DM_SECURE_DATA_FLAG, OFF_DATA_SIZE, OFF_FLAGS, read_u32, require_range, validate_ioctl_params,
    validate_ioctl_size, write_u32,
};
use crate::prelude::*;

/// Executes the copy/validate/dispatch/copyout contract after version negotiation.
///
/// The input capacity is immutable. NO_PARAMS commands only consume the fixed
/// prefix; handlers publish their actual output length separately. Like Linux
/// `copy_params`/`ctl_ioctl`, a normal failure never copies command results out,
/// and a secure request is cleared before validation or any state mutation.
pub(super) fn process_ioctl_parameters(
    command: u8,
    version: &[u8; size_of::<[u32; 3]>()],
    read: impl FnMut(usize, &mut [u8]) -> Result<()>,
    clear: impl FnMut(usize) -> Result<()>,
    write: impl FnMut(&[u8]) -> Result<()>,
    dispatch: impl FnOnce(&mut [u8]) -> Result<()>,
) -> Result<()> {
    process_ioctl_parameters_with_allocator(
        command,
        version,
        read,
        clear,
        write,
        dispatch,
        IoctlBuffer::new,
    )
}

/// Keeps allocation injectable for ENOMEM and post-wipe lifetime assertions.
pub(super) fn process_ioctl_parameters_with_allocator(
    command: u8,
    version: &[u8; size_of::<[u32; 3]>()],
    mut read: impl FnMut(usize, &mut [u8]) -> Result<()>,
    mut clear: impl FnMut(usize) -> Result<()>,
    mut write: impl FnMut(&[u8]) -> Result<()>,
    dispatch: impl FnOnce(&mut [u8]) -> Result<()>,
    allocate: impl FnOnce(usize, bool) -> Result<IoctlBuffer>,
) -> Result<()> {
    let header = read_ioctl_header(version, &mut read)?;
    let input_size = read_u32(&header.bytes, OFF_DATA_SIZE)? as usize;
    validate_ioctl_size(input_size)?;
    let size = if command_has_no_params(command) {
        DM_IOCTL_FIXED_PREFIX_SIZE
    } else {
        input_size
    };
    let secure = read_u32(&header.bytes, OFF_FLAGS)? & DM_SECURE_DATA_FLAG != 0;
    let copied = copy_ioctl_payload_with_allocator(&header.bytes, size, read, allocate);
    let mut buffer = match copied {
        Ok(buffer) => buffer,
        Err(error) => {
            // Linux also clears secure input on allocation failure; a clear
            // fault takes precedence. Partial copy faults do not clear it.
            if secure && error.error() == Errno::ENOMEM {
                clear(size)?;
            }
            return Err(error);
        }
    };
    if secure {
        clear(size)?;
    }
    validate_ioctl_params(command, &mut buffer)?;
    write_u32(&mut buffer, OFF_DATA_SIZE, DM_IOCTL_FIXED_PREFIX_SIZE)?;
    dispatch(&mut buffer)?;
    let output_size = read_u32(&buffer, OFF_DATA_SIZE)? as usize;
    require_range(&buffer, 0, output_size)?;
    if output_size != 0 {
        write(&buffer[..output_size])?;
    }
    Ok(())
}

/// Identifies Linux's fixed-prefix-only control commands.
fn command_has_no_params(command: u8) -> bool {
    matches!(
        command,
        DM_DEV_CREATE_CMD | DM_DEV_REMOVE_CMD | DM_DEV_STATUS_CMD
    )
}

/// Owns the fixed snapshot, including bytes copied before a user read fault.
///
/// Wiping unconditionally also covers errors before the secure flag is decoded.
#[derive(Debug)]
pub(super) struct IoctlHeader {
    pub(super) bytes: [u8; DM_IOCTL_FIXED_PREFIX_SIZE],
    #[cfg(ktest)]
    pub(super) wipe_observer: Option<Arc<Mutex<Option<Vec<u8>>>>>,
}

impl Drop for IoctlHeader {
    fn drop(&mut self) {
        VmWriter::from(self.bytes.as_mut_slice()).fill_zeros(DM_IOCTL_FIXED_PREFIX_SIZE);
        #[cfg(ktest)]
        if let Some(observer) = &self.wipe_observer {
            *observer.lock() = Some(self.bytes.to_vec());
        }
    }
}

/// Captures the 305-byte fixed prefix once, retaining the negotiated version.
///
/// Linux `copy_params` reads only through `offsetof(dm_ioctl, data)`, not the
/// 312-byte aligned structure size. A read fault prevents command dispatch.
pub(super) fn read_ioctl_header(
    version: &[u8; size_of::<[u32; 3]>()],
    read: impl FnMut(usize, &mut [u8]) -> Result<()>,
) -> Result<IoctlHeader> {
    capture_ioctl_header(
        IoctlHeader {
            bytes: [0; DM_IOCTL_FIXED_PREFIX_SIZE],
            #[cfg(ktest)]
            wipe_observer: None,
        },
        version,
        read,
    )
}

/// Takes ownership before reading, so even a partial fault drops the header.
/// Tests attach an observer to the owner without replacing this copy path.
pub(super) fn capture_ioctl_header(
    mut header: IoctlHeader,
    version: &[u8; size_of::<[u32; 3]>()],
    mut read: impl FnMut(usize, &mut [u8]) -> Result<()>,
) -> Result<IoctlHeader> {
    header.bytes[..version.len()].copy_from_slice(version);
    read(version.len(), &mut header.bytes[version.len()..])?;
    Ok(header)
}

/// Owns copied parameters and wipes secure input on every exit path.
///
/// VmWriter uses volatile stores for kernel memory, so disposal cannot be
/// optimized into an ordinary deallocation without the requested wipe.
#[derive(Debug)]
pub(super) struct IoctlBuffer {
    bytes: Vec<u8>,
    secure: bool,
    #[cfg(ktest)]
    pub(super) wipe_observer: Option<Arc<Mutex<Option<Vec<u8>>>>>,
}

impl IoctlBuffer {
    /// Allocates before copying any parameters and installs the wipe policy.
    pub(super) fn new(size: usize, secure: bool) -> Result<Self> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| {
            Error::with_message(Errno::ENOMEM, "cannot allocate ioctl parameter buffer")
        })?;
        bytes.resize(size, 0);
        Ok(Self {
            bytes,
            secure,
            #[cfg(ktest)]
            wipe_observer: None,
        })
    }
}

impl Deref for IoctlBuffer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

impl DerefMut for IoctlBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

impl Drop for IoctlBuffer {
    fn drop(&mut self) {
        if self.secure {
            let len = self.bytes.len();
            VmWriter::from(self.bytes.as_mut_slice()).fill_zeros(len);
        }
        #[cfg(ktest)]
        if let Some(observer) = &self.wipe_observer {
            *observer.lock() = Some(self.bytes.clone());
        }
    }
}

/// Copies only the payload after a validated, immutable header snapshot.
///
/// `data_size` must have passed `validate_ioctl_size`. Allocation and
/// dispatch use the same header even if userspace changes it during this copy.
/// This does not provide an atomic snapshot of the entire userspace request.
#[cfg(ktest)]
pub(super) fn copy_ioctl_payload(
    header: &[u8; DM_IOCTL_FIXED_PREFIX_SIZE],
    data_size: usize,
    read: impl FnMut(usize, &mut [u8]) -> Result<()>,
) -> Result<IoctlBuffer> {
    copy_ioctl_payload_with_allocator(header, data_size, read, IoctlBuffer::new)
}

/// Copies the payload into an owner whose cleanup policy predates copyin.
fn copy_ioctl_payload_with_allocator(
    header: &[u8; DM_IOCTL_FIXED_PREFIX_SIZE],
    data_size: usize,
    mut read: impl FnMut(usize, &mut [u8]) -> Result<()>,
    allocate: impl FnOnce(usize, bool) -> Result<IoctlBuffer>,
) -> Result<IoctlBuffer> {
    let secure = read_u32(header, OFF_FLAGS)? & DM_SECURE_DATA_FLAG != 0;
    let mut buffer = allocate(data_size, secure)?;
    buffer[..header.len()].copy_from_slice(header);
    if data_size > header.len() {
        read(header.len(), &mut buffer[header.len()..])?;
    }
    Ok(buffer)
}
