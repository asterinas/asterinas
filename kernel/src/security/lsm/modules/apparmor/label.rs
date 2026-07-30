// SPDX-License-Identifier: MPL-2.0

//! AppArmor labels carried by process credentials.

use super::{UNCONFINED_PROFILE_NAME, policy};
use crate::{prelude::*, process::posix_thread::PosixThread};

/// An AppArmor label attached to process credentials.
#[derive(Clone, Debug)]
pub(in crate::security::lsm) enum Label {
    /// The task is not confined by AppArmor.
    Unconfined,
    /// The task is confined by the named profile.
    Confined(Arc<str>),
}

pub(super) fn task_profile_name(posix_thread: &PosixThread) -> Option<Arc<str>> {
    match &*posix_thread
        .credentials()
        .security()
        .apparmor_label()
        .read()
    {
        Label::Unconfined => None,
        Label::Confined(profile_name) => Some(profile_name.clone()),
    }
}

pub(super) fn confine_task(posix_thread: &PosixThread, value: &str) -> Result<()> {
    let profile_name = value.trim();
    if profile_name == UNCONFINED_PROFILE_NAME || !policy::is_valid_profile_name(profile_name) {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name is invalid");
    }

    let Some(stored_name) = policy::stored_profile_name(profile_name) else {
        return_errno_with_message!(Errno::ENOENT, "the AppArmor profile is not loaded");
    };

    let credentials = posix_thread.credentials();
    let mut label = credentials.security().apparmor_label().write();
    if !matches!(*label, Label::Unconfined) {
        return_errno_with_message!(
            Errno::EPERM,
            "a confined task cannot change its AppArmor profile"
        );
    }

    *label = Label::Confined(stored_name);
    Ok(())
}
