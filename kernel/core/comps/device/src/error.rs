// SPDX-License-Identifier: MPL-2.0

//! The errors the device model reports, and their conversions to and from
//! `aster_systree::Error`.

use core::fmt;

/// Errors reported by the device model.
#[derive(Debug)]
pub enum Error {
    /// The device has already been added, or is being added.
    AlreadyAdded,
    /// The device has not been added, or has already been removed.
    NotAdded,
    /// The parent device is not currently added.
    ParentNotAdded,
    /// The device still has child devices.
    HasChildren,
    /// A sibling with the same name already exists.
    NameConflict,
    /// The named object does not exist.
    NotFound,
    /// The name is empty or contains `/` or `NUL`.
    InvalidName,
    /// No driver accepted the device.
    NoDriver,
    /// The device is already bound to a driver.
    AlreadyBound,
    /// The device is not bound to a driver.
    NotBound,
    /// The driver rejected the device in `probe`.
    ProbeFailed,
    /// The driver has been unregistered.
    DriverUnregistered,
    /// An attribute callback failed.
    Attribute,
    /// The device has no device number, so it has no `dev` attribute.
    NoDevNum,
    /// Formatting an attribute value failed.
    Format,
    /// The value written to an attribute is invalid.
    InvalidValue,
    /// A resource (such as attribute IDs) is exhausted.
    ResourceUnavailable,
    /// The kernel hook failed to create or delete a device node.
    Hook,
    /// An error from the underlying `SysTree`.
    SysTree(aster_systree::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::AlreadyAdded => write!(f, "the device has already been added"),
            Error::NotAdded => write!(f, "the device has not been added"),
            Error::ParentNotAdded => write!(f, "the parent device has not been added"),
            Error::HasChildren => write!(f, "the device still has child devices"),
            Error::NameConflict => write!(f, "a sibling with the same name exists"),
            Error::NotFound => write!(f, "the object does not exist"),
            Error::InvalidName => write!(f, "the name is invalid"),
            Error::NoDriver => write!(f, "no driver accepted the device"),
            Error::AlreadyBound => write!(f, "the device is already bound to a driver"),
            Error::NotBound => write!(f, "the device is not bound to a driver"),
            Error::ProbeFailed => write!(f, "the driver rejected the device"),
            Error::DriverUnregistered => write!(f, "the driver has been unregistered"),
            Error::NoDevNum => write!(f, "the device has no device number"),
            Error::Format => write!(f, "formatting an attribute value failed"),
            Error::Attribute => write!(f, "attribute operation failed"),
            Error::InvalidValue => write!(f, "invalid attribute value"),
            Error::ResourceUnavailable => write!(f, "resource unavailable"),
            Error::Hook => write!(f, "kernel hook failed"),
            Error::SysTree(e) => write!(f, "systree error: {}", e),
        }
    }
}

impl From<aster_systree::Error> for Error {
    fn from(e: aster_systree::Error) -> Self {
        match e {
            aster_systree::Error::AlreadyExists => Error::NameConflict,
            aster_systree::Error::NotFound => Error::NotFound,
            aster_systree::Error::ResourceUnavailable => Error::ResourceUnavailable,
            other => Error::SysTree(other),
        }
    }
}

impl From<fmt::Error> for Error {
    fn from(_: fmt::Error) -> Self {
        Error::Format
    }
}

impl From<Error> for aster_systree::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::NotFound => aster_systree::Error::NotFound,
            Error::NameConflict => aster_systree::Error::AlreadyExists,
            Error::InvalidValue | Error::InvalidName => aster_systree::Error::InvalidOperation,
            Error::ResourceUnavailable => aster_systree::Error::ResourceUnavailable,
            Error::NotAdded => aster_systree::Error::IsDead,
            Error::SysTree(inner) => inner,
            _ => aster_systree::Error::AttributeError,
        }
    }
}

pub type Result<T, E = Error> = core::result::Result<T, E>;
