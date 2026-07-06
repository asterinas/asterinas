// SPDX-License-Identifier: MPL-2.0

use super::super::modules;
use crate::{
    fs::{file::Permission, vfs::path::Path},
    prelude::*,
};

/// Runs socket creation hooks in module order.
pub fn on_socket_create(context: &SocketCreateContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_socket_create(context)?;
    }

    Ok(())
}

/// Runs socket message hooks in module order.
pub fn on_socket_message(context: &SocketMessageContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_socket_message(context)?;
    }

    Ok(())
}

/// The inputs for labeling a newly created socket.
pub struct SocketCreateContext<'a> {
    path: &'a Path,
}

impl<'a> SocketCreateContext<'a> {
    /// Creates a socket creation context.
    pub const fn new(path: &'a Path) -> Self {
        Self { path }
    }

    /// Returns the socket pseudo path.
    pub const fn path(&self) -> &'a Path {
        self.path
    }
}

/// The inputs for checking socket send and receive operations.
pub struct SocketMessageContext<'a> {
    path: &'a Path,
    permission: Permission,
}

impl<'a> SocketMessageContext<'a> {
    /// Creates a socket message context.
    pub const fn new(path: &'a Path, permission: Permission) -> Self {
        Self { path, permission }
    }

    /// Returns the socket pseudo path.
    pub const fn path(&self) -> &'a Path {
        self.path
    }

    /// Returns the requested permission.
    pub const fn permission(&self) -> Permission {
        self.permission
    }
}
