// SPDX-License-Identifier: MPL-2.0

//! I/O port access.

pub use x86_64::{
    instructions::port::{
        PortReadAccess as IoPortReadAccess, PortWriteAccess as IoPortWriteAccess, ReadOnlyAccess,
        ReadWriteAccess, WriteOnlyAccess,
    },
    structures::port::{PortRead, PortWrite},
};
