// SPDX-License-Identifier: MPL-2.0

//! AppArmor M1 userspace policy loading and query interface.
//!
//! A successful `.load` validates and stores profiles but does not activate
//! policy enforcement. The `profiles` file reports store membership only.

use alloc::vec::Vec;

use aster_core::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::{
        lsm::hooks::{self as lsm_hooks, CapableContext},
        securityfs::SecurityFsFile,
    },
};
use aster_util::printer::VmPrinter;
use ostd::{
    mm::{FallibleVmRead, VmReader, VmWriter},
    task::Task,
};

use crate::policy;

static FILES: [SecurityFsFile; 2] = [
    SecurityFsFile::write_only(".load", write_load),
    SecurityFsFile::read_only("profiles", read_profiles),
];
pub(super) fn register() -> Result<()> {
    aster_core::security::securityfs::register_directory("apparmor", &FILES)
}

fn check_policy_admin() -> Result<()> {
    let task = Task::current().ok_or_else(access_denied)?;
    let posix_thread = task.as_posix_thread().ok_or_else(access_denied)?;

    lsm_hooks::on_capable(CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::MAC_ADMIN,
    ))
    .map_err(|_| access_denied())
}

fn write_load(offset: usize, reader: &mut VmReader) -> Result<usize> {
    check_policy_admin()?;
    if offset != 0 {
        return Err(Error::new(Errno::ESPIPE));
    }

    let len = reader.remain();
    if len == 0 {
        return Err(Error::new(Errno::EINVAL));
    }
    if len > policy::MAX_POLICY_BLOB_SIZE {
        return Err(Error::new(Errno::E2BIG));
    }

    let mut blob = Vec::new();
    blob.try_reserve_exact(len)
        .map_err(|_| Error::new(Errno::ENOMEM))?;
    blob.resize(len, 0);
    let copied = reader
        .read_fallible(&mut VmWriter::from(blob.as_mut_slice()))
        .map_err(|_| Error::new(Errno::EFAULT))?;
    if copied != len {
        return Err(Error::new(Errno::EFAULT));
    }

    policy::POLICY_STORE.load(&blob)?;
    Ok(len)
}

fn read_profiles(offset: usize, writer: &mut VmWriter) -> Result<usize> {
    check_policy_admin()?;
    let snapshot = policy::POLICY_STORE.snapshot();
    let mut printer = VmPrinter::new_skip(writer, offset);

    if let Some(snapshot) = snapshot {
        for profile in snapshot.profiles() {
            printer
                .write_fmt(format_args!(
                    "{} ({})\n",
                    profile.name(),
                    profile.mode().as_str()
                ))
                .map_err(|_| Error::new(Errno::EFAULT))?;
        }
    }

    Ok(printer.bytes_written())
}

fn access_denied() -> Error {
    Error::new(Errno::EACCES)
}
