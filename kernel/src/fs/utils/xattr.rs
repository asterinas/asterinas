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

/// Represents the name of an xattr. It includes both a valid namespace
/// and a full name string slice, which contains the namespace prefix.
///
/// For example, "user.foo" is a valid xattr name, and its namespace
/// is `XattrNamespace::User`.
#[derive(Debug, Hash)]
pub struct XattrName<'a> {
    namespace: XattrNamespace,
    full_name: &'a str,
}

impl XattrNamespace {
    pub fn try_from_full_name(full_name: &str) -> Option<XattrNamespace> {
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

    pub fn is_user(&self) -> bool {
        matches!(self, XattrNamespace::User)
    }

    pub fn is_admin(&self) -> bool {
        matches!(self, XattrNamespace::Trusted)
    }
}

impl<'a> XattrName<'a> {
    pub fn try_from_full_name(full_name: &'a str) -> Option<Self> {
        let namespace = XattrNamespace::try_from_full_name(full_name)?;
        Some(Self {
            namespace,
            full_name,
        })
    }

    pub fn namespace(&self) -> XattrNamespace {
        self.namespace
    }

    pub const fn full_name(&self) -> &'a str {
        self.full_name
    }

    pub const fn full_name_len(&self) -> usize {
        self.full_name.len()
    }
}

bitflags::bitflags! {
    /// Flags for setting an xattr value.
    pub struct XattrSetFlags: u8 {
        /// Creates a new xattr if it doesn't exist, or replaces the value if it does.
        const CREATE_OR_REPLACE = 0;
        /// Creates a new xattr, fails if it already exists.
        const CREATE_ONLY = 1;
        /// Replaces the value of an existing xattr, fails if it doesn't exist.
        const REPLACE_ONLY = 2;
    }
}
