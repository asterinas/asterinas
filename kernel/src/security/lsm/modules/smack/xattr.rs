// SPDX-License-Identifier: MPL-2.0

use super::label::{MAX_LABEL_LEN, SmackLabel};
use crate::{
    fs::vfs::{
        inode::Inode,
        xattr::{XattrName, XattrSetFlags},
    },
    prelude::*,
};

const SMACK64: &str = "security.SMACK64";
const SMACK64EXEC: &str = "security.SMACK64EXEC";
const SMACK64MMAP: &str = "security.SMACK64MMAP";
const SMACK64TRANSMUTE: &str = "security.SMACK64TRANSMUTE";
const SMACK64IPIN: &str = "security.SMACK64IPIN";
const SMACK64IPOUT: &str = "security.SMACK64IPOUT";

/// A Smack-managed xattr.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmackXattr {
    /// The object access-control label.
    Access,
    /// The task label applied after `execve`.
    Exec,
    /// The mmap mediation label.
    Mmap,
    /// The directory transmutation marker.
    Transmute,
    /// The inbound socket packet label.
    IpIn,
    /// The outbound socket packet label.
    IpOut,
}

impl SmackXattr {
    /// Parses a Smack xattr from its full name.
    pub fn from_full_name(full_name: &str) -> Option<Self> {
        match full_name {
            SMACK64 => Some(Self::Access),
            SMACK64EXEC => Some(Self::Exec),
            SMACK64MMAP => Some(Self::Mmap),
            SMACK64TRANSMUTE => Some(Self::Transmute),
            SMACK64IPIN => Some(Self::IpIn),
            SMACK64IPOUT => Some(Self::IpOut),
            _ => None,
        }
    }

    /// Returns whether this xattr carries a Smack label.
    pub const fn carries_label(self) -> bool {
        matches!(
            self,
            Self::Access | Self::Exec | Self::Mmap | Self::IpIn | Self::IpOut
        )
    }
}

/// Returns whether an xattr belongs to Smack.
pub fn is_smack_xattr(full_name: &str) -> bool {
    SmackXattr::from_full_name(full_name).is_some()
}

/// Validates a Smack xattr update.
pub fn validate_update(name: XattrName<'_>, value: &[u8]) -> Result<()> {
    let Some(smack_xattr) = SmackXattr::from_full_name(name.full_name()) else {
        return Ok(());
    };

    if smack_xattr.carries_label() {
        SmackLabel::parse_xattr_value(value)?;
        return Ok(());
    }

    if value == b"TRUE" {
        return Ok(());
    }

    return_errno_with_message!(
        Errno::EINVAL,
        "`security.SMACK64TRANSMUTE` must have the value `TRUE`"
    );
}

/// Returns the Smack access label attached to an inode.
pub fn access_label(inode: &dyn Inode) -> Result<SmackLabel> {
    read_label(inode, SMACK64).map(|label| label.unwrap_or_else(SmackLabel::floor))
}

/// Returns the Smack exec label attached to an inode.
pub fn exec_label(inode: &dyn Inode) -> Result<Option<SmackLabel>> {
    read_label(inode, SMACK64EXEC)
}

/// Returns the Smack mmap label attached to an inode.
pub fn mmap_label(inode: &dyn Inode) -> Result<Option<SmackLabel>> {
    read_label(inode, SMACK64MMAP)
}

/// Returns whether a directory carries the Smack transmute marker.
pub fn is_transmuting_directory(inode: &dyn Inode) -> Result<bool> {
    let Some(xattr_name) = XattrName::try_from_full_name(SMACK64TRANSMUTE) else {
        return_errno_with_message!(Errno::EOPNOTSUPP, "invalid xattr namespace");
    };
    let mut value = vec![0u8; b"TRUE".len()];
    let mut value_writer = VmWriter::from(value.as_mut_slice()).to_fallible();
    match inode.get_xattr(xattr_name, &mut value_writer) {
        Ok(value_len) => Ok(value_len == b"TRUE".len() && value == b"TRUE"),
        Err(error) if is_absent_or_unsupported_xattr_error(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Updates the Smack access label attached to an inode if the filesystem supports it.
pub fn set_access_label(inode: &dyn Inode, label: &SmackLabel) -> Result<()> {
    let Some(xattr_name) = XattrName::try_from_full_name(SMACK64) else {
        return_errno_with_message!(Errno::EOPNOTSUPP, "invalid xattr namespace");
    };
    let mut value_reader = VmReader::from(label.as_str().as_bytes()).to_fallible();

    match inode.set_xattr(xattr_name, &mut value_reader, XattrSetFlags::empty()) {
        Ok(()) => Ok(()),
        Err(error) if is_unsupported_xattr_error(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn read_label(inode: &dyn Inode, name: &'static str) -> Result<Option<SmackLabel>> {
    let Some(xattr_name) = XattrName::try_from_full_name(name) else {
        return_errno_with_message!(Errno::EOPNOTSUPP, "invalid xattr namespace");
    };
    let mut empty = [];
    let mut empty_writer = VmWriter::from(empty.as_mut_slice()).to_fallible();
    let value_len = match inode.get_xattr(xattr_name, &mut empty_writer) {
        Ok(value_len) => value_len,
        Err(error) if is_absent_or_unsupported_xattr_error(&error) => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };

    if value_len > MAX_LABEL_LEN {
        return_errno_with_message!(Errno::EINVAL, "the Smack label xattr is too long");
    }

    let Some(xattr_name) = XattrName::try_from_full_name(name) else {
        return_errno_with_message!(Errno::EOPNOTSUPP, "invalid xattr namespace");
    };
    let mut value = vec![0u8; value_len];
    let mut value_writer = VmWriter::from(value.as_mut_slice()).to_fallible();
    inode.get_xattr(xattr_name, &mut value_writer)?;

    SmackLabel::parse_xattr_value(&value).map(Some)
}

fn is_absent_or_unsupported_xattr_error(error: &Error) -> bool {
    matches!(error.error(), Errno::ENODATA) || is_unsupported_xattr_error(error)
}

fn is_unsupported_xattr_error(error: &Error) -> bool {
    // Ramfs reports unsupported file types, such as symlinks, with `EPERM`.
    matches!(error.error(), Errno::EOPNOTSUPP | Errno::EPERM)
}
