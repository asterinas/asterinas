// SPDX-License-Identifier: MPL-2.0

//! Copied parameter ownership, fault precedence, and secure clearing.

use core::cell::{Cell, RefCell};

use ostd::prelude::ktest;

use super::{
    super::{abi::*, ioctl_params::*},
    request, version,
};
use crate::prelude::*;

#[ktest]
fn preserves_ioctl_header_when_userspace_changes_before_payload_copy() {
    let mut version = [0u8; 12];
    write_version(&mut version).unwrap();
    let mut user = vec![0u8; DM_IOCTL_FIXED_PREFIX_SIZE + 8];
    let size = user.len();
    write_u32(&mut user, OFF_DATA_SIZE, size as u32).unwrap();
    write_u32(&mut user, OFF_DATA_START, DM_IOCTL_FIXED_PREFIX_SIZE as u32).unwrap();
    write_u32(&mut user, OFF_FLAGS, DM_SECURE_DATA_FLAG).unwrap();
    user[OFF_NAME..OFF_NAME + 6].copy_from_slice(b"alpha\0");
    let header = read_ioctl_header(&version, |offset, bytes| {
        assert_eq!(offset, 12);
        assert_eq!(bytes.len(), DM_IOCTL_FIXED_PREFIX_SIZE - 12);
        bytes.copy_from_slice(&user[offset..offset + bytes.len()]);
        Ok(())
    })
    .unwrap();
    validate_ioctl_size(size).unwrap();

    // Mutate every header field after capture, not just the device name.
    user[..DM_IOCTL_FIXED_PREFIX_SIZE].fill(0xff);
    user[DM_IOCTL_FIXED_PREFIX_SIZE..].fill(0x5a);
    let buffer = copy_ioctl_payload(&header.bytes, size, |offset, bytes| {
        assert_eq!(offset, DM_IOCTL_FIXED_PREFIX_SIZE);
        assert_eq!(bytes.len(), 8);
        bytes.copy_from_slice(&user[offset..offset + bytes.len()]);
        Ok(())
    })
    .unwrap();
    assert_eq!(&buffer[..DM_IOCTL_FIXED_PREFIX_SIZE], &header.bytes);
    assert_eq!(&buffer[..12], &version);
    assert_eq!(read_u32(&buffer, OFF_FLAGS).unwrap(), DM_SECURE_DATA_FLAG);
    assert_eq!(&buffer[OFF_NAME..OFF_NAME + 6], b"alpha\0");
    assert_eq!(&buffer[DM_IOCTL_FIXED_PREFIX_SIZE..], &[0x5a; 8]);
}

#[ktest]
fn ioctl_snapshot_propagates_copy_faults() {
    let version = [0u8; 12];
    let header_error = read_ioctl_header(&version, |_, _| {
        Err(Error::with_message(Errno::EFAULT, "injected header fault"))
    })
    .unwrap_err();
    assert_eq!(header_error.error(), Errno::EFAULT);

    let header = [0u8; DM_IOCTL_FIXED_PREFIX_SIZE];
    let mut dispatched = false;
    let result = copy_ioctl_payload(&header, header.len() + 8, |_, _| {
        Err(Error::with_message(Errno::EFAULT, "injected payload fault"))
    })
    .map(|_| dispatched = true);
    assert_eq!(result.unwrap_err().error(), Errno::EFAULT);
    assert!(!dispatched);
}

#[ktest]
fn header_only_ioctl_does_not_read_payload() {
    let header = [0u8; DM_IOCTL_FIXED_PREFIX_SIZE];
    let buffer = copy_ioctl_payload(&header, header.len(), |_, _| {
        panic!("header-only request must not trigger another user read")
    })
    .unwrap();
    assert_eq!(&buffer[..], &header);
}

#[ktest]
fn no_params_reads_and_returns_only_the_fixed_prefix() {
    for command in [DM_DEV_CREATE_CMD, DM_DEV_REMOVE_CMD, DM_DEV_STATUS_CMD] {
        let mut input = request(DM_IOCTL_FIXED_PREFIX_SIZE);
        write_c_string_fixed(&mut input, OFF_NAME, DM_NAME_LEN, "v5-prefix").unwrap();
        write_u32(&mut input, OFF_DATA_SIZE, DM_IOCTL_MAX_SIZE).unwrap();
        write_u32(&mut input, OFF_DATA_START, u32::MAX).unwrap();
        let reads = Cell::new(0);
        let writes = Cell::new(0);
        process_ioctl_parameters(
            command,
            &version(),
            |offset, bytes| {
                reads.set(reads.get() + 1);
                assert_eq!(offset, 12);
                assert_eq!(bytes.len(), 293);
                bytes.copy_from_slice(&input[offset..]);
                Ok(())
            },
            |_| panic!("nonsecure input must not be cleared"),
            |bytes| {
                writes.set(writes.get() + 1);
                assert_eq!(bytes.len(), 305);
                assert_eq!(read_u32(bytes, OFF_DATA_SIZE).unwrap(), 305);
                Ok(())
            },
            |bytes| {
                assert_eq!(bytes.len(), 305);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(reads.get(), 1);
        assert_eq!(writes.get(), 1);
    }
}

#[ktest]
fn secure_copy_faults_and_dispatch_exits_wipe_owned_parameters() {
    // 0: success, 1: partial copyin, 2: clear fault,
    // 4: handler error, 5: copyout fault. Observe bytes before deallocation.
    for fault in [0, 1, 2, 4, 5] {
        let mut input = request(384);
        write_u32(&mut input, OFF_FLAGS, DM_SECURE_DATA_FLAG).unwrap();
        input[305..].fill(0xa5);
        let input = RefCell::new(input);
        let cleared = Cell::new(false);
        let dispatched = Cell::new(false);
        let written = Cell::new(false);
        let observed = Arc::new(Mutex::new(None));
        let result = process_ioctl_parameters_with_allocator(
            DM_LIST_DEVICES_CMD,
            &version(),
            |offset, bytes| {
                if offset == 305 && fault == 1 {
                    bytes[..3].fill(0xa5);
                    return Err(Error::new(Errno::EFAULT));
                }
                bytes.copy_from_slice(&input.borrow()[offset..offset + bytes.len()]);
                Ok(())
            },
            |len| {
                cleared.set(true);
                assert_eq!(len, 384);
                if fault == 2 {
                    return Err(Error::new(Errno::EFAULT));
                }
                input.borrow_mut()[..len].fill(0);
                Ok(())
            },
            |bytes| {
                written.set(true);
                assert!(cleared.get());
                assert!(dispatched.get());
                assert_eq!(bytes.len(), 305);
                if fault == 5 {
                    return Err(Error::new(Errno::EFAULT));
                }
                input.borrow_mut()[..bytes.len()].copy_from_slice(bytes);
                Ok(())
            },
            |bytes| {
                dispatched.set(true);
                assert!(cleared.get());
                assert_eq!(read_u32(bytes, OFF_FLAGS)? & DM_SECURE_DATA_FLAG, 0);
                if fault == 4 {
                    return Err(Error::new(Errno::EINVAL));
                }
                Ok(())
            },
            |size, secure| {
                let mut buffer = IoctlBuffer::new(size, secure)?;
                buffer.wipe_observer = Some(observed.clone());
                Ok(buffer)
            },
        );
        if fault == 0 {
            result.unwrap();
        } else {
            let expected = if fault == 4 {
                Errno::EINVAL
            } else {
                Errno::EFAULT
            };
            assert_eq!(result.unwrap_err().error(), expected);
        }
        assert_eq!(cleared.get(), fault != 1);
        assert_eq!(dispatched.get(), matches!(fault, 0 | 4 | 5));
        assert_eq!(written.get(), matches!(fault, 0 | 5));
        let observed = observed.lock();
        let bytes = observed.as_ref().unwrap();
        assert_eq!(bytes.len(), 384);
        assert!(bytes.iter().all(|byte| *byte == 0));
        if !matches!(fault, 1 | 2) {
            assert!(input.borrow()[305..].iter().all(|byte| *byte == 0));
        }
    }
}

#[ktest]
fn allocation_failure_obeys_secure_clear_and_errno_precedence() {
    for secure in [false, true] {
        for clear_fails in [false, true] {
            let mut input = request(384);
            write_u32(
                &mut input,
                OFF_FLAGS,
                if secure { DM_SECURE_DATA_FLAG } else { 0 },
            )
            .unwrap();
            let cleared = Cell::new(false);
            let result = process_ioctl_parameters_with_allocator(
                DM_LIST_DEVICES_CMD,
                &version(),
                |offset, bytes| {
                    assert_eq!(offset, 12);
                    bytes.copy_from_slice(&input[offset..offset + bytes.len()]);
                    Ok(())
                },
                |len| {
                    assert_eq!(len, input.len());
                    cleared.set(true);
                    if clear_fails {
                        Err(Error::new(Errno::EFAULT))
                    } else {
                        Ok(())
                    }
                },
                |_| panic!("allocation failed"),
                |_| panic!("allocation failed"),
                |_, _| Err(Error::new(Errno::ENOMEM)),
            );
            assert_eq!(cleared.get(), secure);
            assert_eq!(
                result.unwrap_err().error(),
                if secure && clear_fails {
                    Errno::EFAULT
                } else {
                    Errno::ENOMEM
                }
            );
        }
    }
}

#[ktest]
fn secure_no_params_failure_clears_only_the_fixed_prefix() {
    for allocation_fails in [false, true] {
        let mut input = request(DM_IOCTL_HEADER_SIZE);
        write_u32(&mut input, OFF_DATA_SIZE, DM_IOCTL_MAX_SIZE).unwrap();
        write_u32(&mut input, OFF_FLAGS, DM_SECURE_DATA_FLAG).unwrap();
        input[DM_IOCTL_FIXED_PREFIX_SIZE..].fill(0xa5);
        let input = RefCell::new(input);
        let cleared = Cell::new(false);
        let observed = Arc::new(Mutex::new(None));
        let result = process_ioctl_parameters_with_allocator(
            DM_DEV_CREATE_CMD,
            &version(),
            |offset, bytes| {
                assert_eq!(offset, 12);
                assert_eq!(offset + bytes.len(), DM_IOCTL_FIXED_PREFIX_SIZE);
                bytes.copy_from_slice(&input.borrow()[offset..offset + bytes.len()]);
                Ok(())
            },
            |len| {
                assert_eq!(len, DM_IOCTL_FIXED_PREFIX_SIZE);
                cleared.set(true);
                input.borrow_mut()[..len].fill(0);
                Ok(())
            },
            |_| panic!("failed secure create must not copyout"),
            |_| panic!("empty create name must fail before dispatch"),
            |size, secure| {
                assert_eq!(size, DM_IOCTL_FIXED_PREFIX_SIZE);
                assert!(secure);
                if allocation_fails {
                    return Err(Error::new(Errno::ENOMEM));
                }
                let mut buffer = IoctlBuffer::new(size, secure)?;
                buffer.wipe_observer = Some(observed.clone());
                Ok(buffer)
            },
        );
        assert_eq!(
            result.unwrap_err().error(),
            if allocation_fails {
                Errno::ENOMEM
            } else {
                Errno::EINVAL
            }
        );
        assert!(cleared.get());
        let input = input.borrow();
        assert!(
            input[..DM_IOCTL_FIXED_PREFIX_SIZE]
                .iter()
                .all(|byte| *byte == 0)
        );
        assert!(
            input[DM_IOCTL_FIXED_PREFIX_SIZE..]
                .iter()
                .all(|byte| *byte == 0xa5)
        );
        let observed = observed.lock();
        if allocation_fails {
            assert!(observed.is_none());
        } else {
            let bytes = observed.as_ref().unwrap();
            assert_eq!(bytes.len(), DM_IOCTL_FIXED_PREFIX_SIZE);
            assert!(bytes.iter().all(|byte| *byte == 0));
        }
    }
}

#[ktest]
fn ordinary_failure_does_not_copyout_or_clear_input() {
    let input = request(384);
    let result = process_ioctl_parameters(
        DM_LIST_DEVICES_CMD,
        &version(),
        |offset, bytes| {
            bytes.copy_from_slice(&input[offset..offset + bytes.len()]);
            Ok(())
        },
        |_| panic!("ordinary input must remain intact"),
        |_| panic!("failed handler must not copyout"),
        |_| Err(Error::new(Errno::EINVAL)),
    );
    assert_eq!(result.unwrap_err().error(), Errno::EINVAL);
}

#[ktest]
fn fixed_fields_are_normalized_only_for_commands_that_use_them() {
    let mut input = request(305);
    input[OFF_NAME..OFF_NAME + DM_NAME_LEN].fill(b'n');
    input[OFF_UUID..OFF_UUID + DM_UUID_LEN].fill(b'u');
    validate_ioctl_params(DM_DEV_CREATE_CMD, &mut input).unwrap();
    assert_eq!(
        required_c_string(&input, OFF_NAME, DM_NAME_LEN)
            .unwrap()
            .len(),
        127
    );
    assert_eq!(
        required_c_string(&input, OFF_UUID, DM_UUID_LEN)
            .unwrap()
            .len(),
        128
    );
    input[OFF_NAME..OFF_NAME + DM_NAME_LEN].fill(0xff);
    input[OFF_UUID..OFF_UUID + DM_UUID_LEN].fill(0xff);
    validate_ioctl_params(DM_LIST_DEVICES_CMD, &mut input).unwrap();
    assert_eq!(input[OFF_NAME + DM_NAME_LEN - 1], 0xff);
    assert_eq!(input[OFF_UUID + DM_UUID_LEN - 1], 0xff);
    for size in [305, 311, 312, DM_IOCTL_MAX_SIZE] {
        validate_ioctl_size(size).unwrap();
    }
    for size in [0, 304, DM_IOCTL_MAX_SIZE + 1, u32::MAX as usize] {
        assert_eq!(
            validate_ioctl_size(size).unwrap_err().error(),
            Errno::EINVAL
        );
    }
}

#[ktest]
fn fixed_header_is_wiped_after_success_and_partial_read_faults() {
    for copied in [0, 8, 32, DM_IOCTL_FIXED_PREFIX_SIZE - 12] {
        let observed = Arc::new(Mutex::new(None));
        let header = IoctlHeader {
            bytes: [0; DM_IOCTL_FIXED_PREFIX_SIZE],
            wipe_observer: Some(observed.clone()),
        };
        let result = capture_ioctl_header(header, &version(), |offset, bytes| {
            assert_eq!(offset, 12);
            bytes[..copied].fill(0xa5);
            if copied == bytes.len() {
                Ok(())
            } else {
                Err(Error::new(Errno::EFAULT))
            }
        });
        if copied == DM_IOCTL_FIXED_PREFIX_SIZE - 12 {
            let header = result.unwrap();
            assert!(observed.lock().is_none());
            assert_eq!(&header.bytes[..12], &version());
            assert!(header.bytes[12..].iter().all(|byte| *byte == 0xa5));
            drop(header);
        } else {
            assert_eq!(result.unwrap_err().error(), Errno::EFAULT);
        }
        let observed = observed.lock();
        let bytes = observed.as_ref().unwrap();
        assert_eq!(bytes.len(), DM_IOCTL_FIXED_PREFIX_SIZE);
        assert!(bytes.iter().all(|byte| *byte == 0));
    }
}

#[ktest]
fn partial_fixed_header_fault_never_allocates_or_dispatches() {
    let result = process_ioctl_parameters_with_allocator(
        DM_LIST_DEVICES_CMD,
        &version(),
        |offset, bytes| {
            assert_eq!(offset, 12);
            bytes[..32].fill(0xa5);
            Err(Error::new(Errno::EFAULT))
        },
        |_| panic!("incomplete header must not clear user memory"),
        |_| panic!("incomplete header must not copyout"),
        |_| panic!("incomplete header must not dispatch"),
        |_, _| panic!("incomplete header must not allocate payload"),
    );
    assert_eq!(result.unwrap_err().error(), Errno::EFAULT);
}
