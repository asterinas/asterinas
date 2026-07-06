// SPDX-License-Identifier: MPL-2.0

use super::super::modules;
use crate::{
    fs::vfs::{inode::Inode, path::Path},
    prelude::*,
};

/// Runs path creation hooks in module order.
pub fn on_path_create(context: &PathCreateContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_path_create(context)?;
    }

    Ok(())
}

/// Runs post-creation path hooks in module order.
pub fn on_path_post_create(context: &PathPostCreateContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_path_post_create(context)?;
    }

    Ok(())
}

/// Runs link hooks in module order.
pub fn on_path_link(context: &PathLinkContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_path_link(context)?;
    }

    Ok(())
}

/// Runs unlink and rmdir hooks in module order.
pub fn on_path_unlink(context: &PathUnlinkContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_path_unlink(context)?;
    }

    Ok(())
}

/// Runs rename hooks in module order.
pub fn on_path_rename(context: &PathRenameContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_path_rename(context)?;
    }

    Ok(())
}

/// Runs metadata update hooks in module order.
pub fn on_path_setattr(context: &PathSetattrContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_path_setattr(context)?;
    }

    Ok(())
}

/// The inputs for checking creation under a directory path.
pub struct PathCreateContext<'a> {
    parent: &'a Path,
}

impl<'a> PathCreateContext<'a> {
    /// Creates a path creation context.
    pub const fn new(parent: &'a Path) -> Self {
        Self { parent }
    }

    /// Returns the parent directory path.
    pub const fn parent(&self) -> &'a Path {
        self.parent
    }
}

/// The inputs for labeling a newly created path.
pub struct PathPostCreateContext<'a> {
    parent: &'a Path,
    child: &'a Path,
}

impl<'a> PathPostCreateContext<'a> {
    /// Creates a post-creation context.
    pub const fn new(parent: &'a Path, child: &'a Path) -> Self {
        Self { parent, child }
    }

    /// Returns the parent directory path.
    pub const fn parent(&self) -> &'a Path {
        self.parent
    }

    /// Returns the newly created path.
    pub const fn child(&self) -> &'a Path {
        self.child
    }
}

/// The inputs for checking a hard link creation.
pub struct PathLinkContext<'a> {
    old_path: &'a Path,
    new_parent: &'a Path,
}

impl<'a> PathLinkContext<'a> {
    /// Creates a path link context.
    pub const fn new(old_path: &'a Path, new_parent: &'a Path) -> Self {
        Self {
            old_path,
            new_parent,
        }
    }

    /// Returns the source path.
    pub const fn old_path(&self) -> &'a Path {
        self.old_path
    }

    /// Returns the destination parent directory.
    pub const fn new_parent(&self) -> &'a Path {
        self.new_parent
    }
}

/// The inputs for checking unlink or rmdir.
pub struct PathUnlinkContext<'a> {
    parent: &'a Path,
    child: &'a dyn Inode,
}

impl<'a> PathUnlinkContext<'a> {
    /// Creates a path unlink context.
    pub const fn new(parent: &'a Path, child: &'a dyn Inode) -> Self {
        Self { parent, child }
    }

    /// Returns the parent directory path.
    pub const fn parent(&self) -> &'a Path {
        self.parent
    }

    /// Returns the child inode being removed.
    pub const fn child(&self) -> &'a dyn Inode {
        self.child
    }
}

/// The inputs for checking rename.
pub struct PathRenameContext<'a> {
    old_parent: &'a Path,
    old_child: &'a dyn Inode,
    new_parent: &'a Path,
    new_child: Option<&'a dyn Inode>,
}

impl<'a> PathRenameContext<'a> {
    /// Creates a path rename context.
    pub const fn new(
        old_parent: &'a Path,
        old_child: &'a dyn Inode,
        new_parent: &'a Path,
        new_child: Option<&'a dyn Inode>,
    ) -> Self {
        Self {
            old_parent,
            old_child,
            new_parent,
            new_child,
        }
    }

    /// Returns the source parent directory.
    pub const fn old_parent(&self) -> &'a Path {
        self.old_parent
    }

    /// Returns the inode being renamed.
    pub const fn old_child(&self) -> &'a dyn Inode {
        self.old_child
    }

    /// Returns the destination parent directory.
    pub const fn new_parent(&self) -> &'a Path {
        self.new_parent
    }

    /// Returns the overwritten child inode, if one exists.
    pub const fn new_child(&self) -> Option<&'a dyn Inode> {
        self.new_child
    }
}

/// The inputs for checking metadata updates.
pub struct PathSetattrContext<'a> {
    path: &'a Path,
}

impl<'a> PathSetattrContext<'a> {
    /// Creates a path metadata update context.
    pub const fn new(path: &'a Path) -> Self {
        Self { path }
    }

    /// Returns the path whose metadata is being updated.
    pub const fn path(&self) -> &'a Path {
        self.path
    }
}
