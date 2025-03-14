// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

pub const XATTR_NAME_MAX_LEN: usize = 255;
pub const XATTR_VALUE_MAX_LEN: usize = 65536;
pub const XATTR_LIST_MAX_LEN: usize = 65536;

/// Represents different namespaces for extended attributes (xattrs).
#[derive(Debug, PartialEq, Eq, Clone, Copy, TryFromInt, Hash)]
#[repr(u8)]
pub enum XattrNamespace {
    User = 1,
    Trusted = 2,
    System = 3,
    Security = 4,
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
}

bitflags::bitflags! {
    pub struct XattrFlags: u8 {
        const XATTR_CREATE_OR_REPLACE = 0;
        const XATTR_CREATE = 1;
        const XATTR_REPLACE = 2;
    }
}

/// A trait for defining xattr values. A xattr value could be any object
/// as long as it can be serialized into bytes.
pub trait XattrValue: Any + Send + Sync {
    fn as_bytes(&self) -> &[u8];

    fn to_bytes(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}
