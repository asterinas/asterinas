// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

pub const XATTR_NAME_MAX_LEN: usize = 255;
pub const XATTR_VALUE_MAX_LEN: usize = 65536;
pub const XATTR_LIST_MAX_LEN: usize = 65536;

/// Represents different namespaces with different capabilities
/// for extended attributes (xattrs).
#[derive(Debug, PartialEq, Eq, Clone, Copy, TryFromInt, Hash)]
#[repr(u8)]
pub enum XattrNamespace {
    User = 1,
    Trusted = 2,
    System = 3,
    Security = 4,
    // More namespaces can be added here.
}

impl XattrNamespace {
    pub fn try_from_name(name: &str) -> Option<XattrNamespace> {
        const USER_PREFIX: &str = "user.";
        const TRUSTED_PREFIX: &str = "trusted.";
        const SYSTEM_PREFIX: &str = "system.";
        const SECURITY_PREFIX: &str = "security.";

        if name.starts_with(USER_PREFIX) {
            Some(XattrNamespace::User)
        } else if name.starts_with(TRUSTED_PREFIX) {
            Some(XattrNamespace::Trusted)
        } else if name.starts_with(SYSTEM_PREFIX) {
            Some(XattrNamespace::System)
        } else if name.starts_with(SECURITY_PREFIX) {
            Some(XattrNamespace::Security)
        } else {
            None
        }
    }

    pub fn is_user(&self) -> bool {
        matches!(self, XattrNamespace::User)
    }

    pub fn is_admin(&self) -> bool {
        matches!(self, XattrNamespace::Trusted)
    }
}

bitflags::bitflags! {
    /// Flags for setting an xattr value.
    pub struct XattrFlags: u8 {
        /// Creates a new xattr if it doesn't exist, or replaces the value if it does.
        const XATTR_CREATE_OR_REPLACE = 0;
        /// Creates a new xattr, fails if it already exists.
        const XATTR_CREATE = 1;
        /// Replaces the value of an existing xattr, fails if it doesn't exist.
        const XATTR_REPLACE = 2;
    }
}
