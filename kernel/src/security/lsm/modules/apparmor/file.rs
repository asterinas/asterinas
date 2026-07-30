// SPDX-License-Identifier: MPL-2.0

//! AppArmor file-open mediation.

use super::{label, policy};
use crate::{
    fs::vfs::path::AbsPathResult,
    prelude::*,
    security::lsm::hooks::{FileOpenAccess, FileOpenContext},
};

pub(super) fn open(context: &FileOpenContext<'_>) -> Result<()> {
    if !context.is_regular_file() {
        return Ok(());
    }

    let Some(profile_name) = label::task_profile_name(context.posix_thread()) else {
        return Ok(());
    };
    let requested = context.requested_access();
    if requested.is_empty() {
        return Ok(());
    }

    let path_name = match context.resolve_path_name() {
        AbsPathResult::Reachable(path_name) => path_name,
        AbsPathResult::Unreachable(path_name) => {
            return deny(&profile_name, &path_name, requested);
        }
    };

    if !policy::allows_file(&profile_name, &path_name, requested) {
        return deny(&profile_name, &path_name, requested);
    }

    Ok(())
}

fn deny(profile_name: &str, path_name: &str, requested: FileOpenAccess) -> Result<()> {
    warn!(
        "apparmor=\"DENIED\" operation=\"file_open\" profile=\"{}\" path={:?} requested={:?}",
        profile_name, path_name, requested
    );
    Err(Error::with_message(
        Errno::EACCES,
        "AppArmor denied file open",
    ))
}
