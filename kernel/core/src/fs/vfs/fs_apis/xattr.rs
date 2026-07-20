// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        file::{InodeMode, InodeType},
        vfs::inode::Inode,
    },
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
};

pub(crate) const XATTR_NAME_MAX_LEN: usize = 255;
pub(crate) const XATTR_VALUE_MAX_LEN: usize = 65536;
pub(crate) const XATTR_LIST_MAX_LEN: usize = 65536;
pub(crate) const SECURITY_CAPABILITY_XATTR_NAME: &str = "security.capability";

/// Clears file privileges after an operation modifies file contents.
pub(in crate::fs) fn clear_file_priv(inode: &dyn Inode) -> Result<()> {
    if inode.type_() != InodeType::File {
        return Ok(());
    }

    let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
    match inode.remove_xattr(xattr_name) {
        Ok(()) => Ok(()),
        Err(error) if matches!(error.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => Ok(()),
        Err(error) => Err(error),
    }?;

    clear_set_id_bits(inode)
}

fn clear_set_id_bits(inode: &dyn Inode) -> Result<()> {
    let mut mode = inode.mode()?;
    if !mode.intersects(InodeMode::S_ISUID | InodeMode::S_ISGID) {
        return Ok(());
    }

    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();

    // Callers with `CAP_FSETID` may preserve both set-ID bits across content changes.
    if lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::FSETID,
    ))
    .is_ok()
    {
        return Ok(());
    }

    let original_mode = mode;
    mode.remove(InodeMode::S_ISUID);

    if mode.contains(InodeMode::S_ISGID) {
        let inode_gid = inode.group()?;
        let credentials = posix_thread.credentials();
        let retain_sgid = !mode.contains(InodeMode::S_IXGRP)
            && (credentials.fsgid() == inode_gid || credentials.groups().contains(&inode_gid));

        if !retain_sgid {
            mode.remove(InodeMode::S_ISGID);
        }
    }

    if mode == original_mode {
        return Ok(());
    }
    inode.set_mode(mode)
}

/// Represents different namespaces with different capabilities
/// for extended attributes (xattrs).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, TryFromInt)]
pub(crate) enum XattrNamespace {
    User = 1,
    Trusted = 2,
    System = 3,
    Security = 4,
    // More namespaces can be added here.
}

/// Represents the name of an xattr. It includes both a valid namespace
/// and a full name string slice, which contains the namespace prefix.
///
/// For example, "user.foo" is a valid xattr name, and its namespace
/// is `XattrNamespace::User`.
#[derive(Debug, Hash)]
pub(crate) struct XattrName<'a> {
    namespace: XattrNamespace,
    full_name: &'a str,
}

impl XattrNamespace {
    pub(crate) fn try_from_full_name(full_name: &str) -> Option<XattrNamespace> {
        const USER_PREFIX: &str = "user.";
        const TRUSTED_PREFIX: &str = "trusted.";
        const SYSTEM_PREFIX: &str = "system.";
        const SECURITY_PREFIX: &str = "security.";

        if full_name.starts_with(USER_PREFIX) {
            Some(XattrNamespace::User)
        } else if full_name.starts_with(TRUSTED_PREFIX) {
            Some(XattrNamespace::Trusted)
        } else if full_name.starts_with(SYSTEM_PREFIX) {
            Some(XattrNamespace::System)
        } else if full_name.starts_with(SECURITY_PREFIX) {
            Some(XattrNamespace::Security)
        } else {
            None
        }
    }

    pub(crate) fn is_user(&self) -> bool {
        matches!(self, XattrNamespace::User)
    }
}

impl<'a> XattrName<'a> {
    pub(crate) fn try_from_full_name(full_name: &'a str) -> Option<Self> {
        let namespace = XattrNamespace::try_from_full_name(full_name)?;
        Some(Self {
            namespace,
            full_name,
        })
    }

    pub(crate) fn namespace(&self) -> XattrNamespace {
        self.namespace
    }

    pub(crate) const fn full_name(&self) -> &'a str {
        self.full_name
    }

    pub(crate) const fn full_name_len(&self) -> usize {
        self.full_name.len()
    }
}

bitflags::bitflags! {
    /// Flags for setting an xattr value.
    pub(crate) struct XattrSetFlags: u8 {
        /// Creates a new xattr if it doesn't exist, or replaces the value if it does.
        const CREATE_OR_REPLACE = 0;
        /// Creates a new xattr, fails if it already exists.
        const CREATE_ONLY = 1;
        /// Replaces the value of an existing xattr, fails if it doesn't exist.
        const REPLACE_ONLY = 2;
    }
}
