// SPDX-License-Identifier: MPL-2.0

use super::super::modules;
use crate::{
    fs::{
        file::{AccessMode, StatusFlags},
        vfs::path::{Path, PathResolver},
    },
    prelude::*,
};

/// Runs file open hooks in module order.
pub fn on_file_open(context: &FileOpenContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_file_open(context)?;
    }

    Ok(())
}

/// Runs file creation hooks in module order.
pub fn on_file_create(context: &FileCreateContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_file_create(context)?;
    }

    Ok(())
}

/// Runs file deletion hooks in module order.
pub fn on_file_delete(context: &FileDeleteContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_file_delete(context)?;
    }

    Ok(())
}

/// Runs file link hooks in module order.
pub fn on_file_link(context: &FileLinkContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_file_link(context)?;
    }

    Ok(())
}

/// Runs file rename hooks in module order.
pub fn on_file_rename(context: &FileRenameContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_file_rename(context)?;
    }

    Ok(())
}

/// Runs file attribute-change hooks in module order.
pub fn on_file_setattr(context: &FileSetattrContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_file_setattr(context)?;
    }

    Ok(())
}

/// The inputs for checking a newly opened file.
pub struct FileOpenContext<'a> {
    path: &'a Path,
    path_resolver: &'a PathResolver,
    access_mode: AccessMode,
    status_flags: StatusFlags,
}

impl<'a> FileOpenContext<'a> {
    /// Creates a file open context.
    pub const fn new(
        path: &'a Path,
        path_resolver: &'a PathResolver,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Self {
        Self {
            path,
            path_resolver,
            access_mode,
            status_flags,
        }
    }

    /// Returns the path being opened.
    pub const fn path(&self) -> &'a Path {
        self.path
    }

    /// Returns the resolver that defines the caller-visible path namespace.
    pub const fn path_resolver(&self) -> &'a PathResolver {
        self.path_resolver
    }

    /// Returns the requested access mode.
    pub const fn access_mode(&self) -> AccessMode {
        self.access_mode
    }

    /// Returns the open status flags.
    pub const fn status_flags(&self) -> StatusFlags {
        self.status_flags
    }
}

/// The kind of filesystem object being created.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileCreateKind {
    /// A regular file.
    Regular,
    /// A directory.
    Directory,
    /// A device node.
    Device,
    /// A named pipe.
    Fifo,
    /// A symbolic link.
    Symlink,
}

/// The inputs for checking a file that will be created and opened.
pub struct FileCreateContext<'a> {
    parent: &'a Path,
    name: &'a str,
    path_resolver: &'a PathResolver,
    kind: FileCreateKind,
    access_mode: Option<AccessMode>,
    status_flags: StatusFlags,
}

impl<'a> FileCreateContext<'a> {
    /// Creates a file creation context.
    pub const fn new(
        parent: &'a Path,
        name: &'a str,
        path_resolver: &'a PathResolver,
        kind: FileCreateKind,
        access_mode: Option<AccessMode>,
        status_flags: StatusFlags,
    ) -> Self {
        Self {
            parent,
            name,
            path_resolver,
            kind,
            access_mode,
            status_flags,
        }
    }

    /// Returns the parent directory path.
    pub const fn parent(&self) -> &'a Path {
        self.parent
    }

    /// Returns the basename that will be created.
    pub const fn name(&self) -> &'a str {
        self.name
    }

    /// Returns the resolver that defines the caller-visible path namespace.
    pub const fn path_resolver(&self) -> &'a PathResolver {
        self.path_resolver
    }

    /// Returns the kind of filesystem object being created.
    pub const fn kind(&self) -> FileCreateKind {
        self.kind
    }

    /// Returns the requested access mode for the opened file.
    pub const fn access_mode(&self) -> Option<AccessMode> {
        self.access_mode
    }

    /// Returns the open status flags.
    pub const fn status_flags(&self) -> StatusFlags {
        self.status_flags
    }
}

/// The kind of filesystem object being deleted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDeleteKind {
    /// A non-directory name removed by `unlink`.
    NonDirectory,
    /// A directory removed by `rmdir`.
    Directory,
}

/// The kind of file attribute being changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSetattrKind {
    /// File mode bits.
    Mode,
    /// File owner or group.
    Owner,
    /// File size.
    Size,
    /// File timestamps.
    Times,
}

/// The inputs for checking a file attribute change.
pub struct FileSetattrContext<'a> {
    path: &'a Path,
    path_resolver: &'a PathResolver,
    kind: FileSetattrKind,
}

impl<'a> FileSetattrContext<'a> {
    /// Creates a file attribute-change context.
    pub const fn new(
        path: &'a Path,
        path_resolver: &'a PathResolver,
        kind: FileSetattrKind,
    ) -> Self {
        Self {
            path,
            path_resolver,
            kind,
        }
    }

    /// Returns the path whose attributes will change.
    pub const fn path(&self) -> &'a Path {
        self.path
    }

    /// Returns the resolver that defines the caller-visible path namespace.
    pub const fn path_resolver(&self) -> &'a PathResolver {
        self.path_resolver
    }

    /// Returns the kind of attribute being changed.
    pub const fn kind(&self) -> FileSetattrKind {
        self.kind
    }
}

/// The inputs for checking a file deletion.
pub struct FileDeleteContext<'a> {
    parent: &'a Path,
    name: &'a str,
    path_resolver: &'a PathResolver,
    kind: FileDeleteKind,
}

impl<'a> FileDeleteContext<'a> {
    /// Creates a file deletion context.
    pub const fn new(
        parent: &'a Path,
        name: &'a str,
        path_resolver: &'a PathResolver,
        kind: FileDeleteKind,
    ) -> Self {
        Self {
            parent,
            name,
            path_resolver,
            kind,
        }
    }

    /// Returns the parent directory path.
    pub const fn parent(&self) -> &'a Path {
        self.parent
    }

    /// Returns the basename that will be deleted.
    pub const fn name(&self) -> &'a str {
        self.name
    }

    /// Returns the resolver that defines the caller-visible path namespace.
    pub const fn path_resolver(&self) -> &'a PathResolver {
        self.path_resolver
    }

    /// Returns the kind of filesystem object being deleted.
    pub const fn kind(&self) -> FileDeleteKind {
        self.kind
    }
}

/// The inputs for checking a hard-link creation.
pub struct FileLinkContext<'a> {
    source: &'a Path,
    target_parent: &'a Path,
    target_name: &'a str,
    path_resolver: &'a PathResolver,
}

impl<'a> FileLinkContext<'a> {
    /// Creates a file link context.
    pub const fn new(
        source: &'a Path,
        target_parent: &'a Path,
        target_name: &'a str,
        path_resolver: &'a PathResolver,
    ) -> Self {
        Self {
            source,
            target_parent,
            target_name,
            path_resolver,
        }
    }

    /// Returns the existing source path.
    pub const fn source(&self) -> &'a Path {
        self.source
    }

    /// Returns the target parent directory path.
    pub const fn target_parent(&self) -> &'a Path {
        self.target_parent
    }

    /// Returns the target basename.
    pub const fn target_name(&self) -> &'a str {
        self.target_name
    }

    /// Returns the resolver that defines the caller-visible path namespace.
    pub const fn path_resolver(&self) -> &'a PathResolver {
        self.path_resolver
    }
}

/// The inputs for checking a rename operation.
pub struct FileRenameContext<'a> {
    source: &'a Path,
    old_parent: &'a Path,
    old_name: &'a str,
    new_parent: &'a Path,
    new_name: &'a str,
    path_resolver: &'a PathResolver,
}

impl<'a> FileRenameContext<'a> {
    /// Creates a file rename context.
    pub const fn new(
        source: &'a Path,
        old_parent: &'a Path,
        old_name: &'a str,
        new_parent: &'a Path,
        new_name: &'a str,
        path_resolver: &'a PathResolver,
    ) -> Self {
        Self {
            source,
            old_parent,
            old_name,
            new_parent,
            new_name,
            path_resolver,
        }
    }

    /// Returns the source path.
    pub const fn source(&self) -> &'a Path {
        self.source
    }

    /// Returns the source parent directory path.
    pub const fn old_parent(&self) -> &'a Path {
        self.old_parent
    }

    /// Returns the source basename.
    pub const fn old_name(&self) -> &'a str {
        self.old_name
    }

    /// Returns the target parent directory path.
    pub const fn new_parent(&self) -> &'a Path {
        self.new_parent
    }

    /// Returns the target basename.
    pub const fn new_name(&self) -> &'a str {
        self.new_name
    }

    /// Returns the resolver that defines the caller-visible path namespace.
    pub const fn path_resolver(&self) -> &'a PathResolver {
        self.path_resolver
    }
}
