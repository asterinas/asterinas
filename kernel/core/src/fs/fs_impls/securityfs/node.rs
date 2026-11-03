// SPDX-License-Identifier: MPL-2.0

//! Immutable securityfs directory and file-node model.
//!
//! [`SecurityRootNode`] contains recursive [`SecurityDirectory`] instances.
//! Each directory maps static file declarations into SysTree attributes and
//! child directory declarations into branch nodes consumed by the inode adapter.

use aster_systree::{
    BranchNodeFields, Error as SysTreeError, Result as SysTreeResult, SysAttrSet,
    SysAttrSetBuilder, SysBranchNode, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
};
use inherit_methods_macro::inherit_methods;
use spin::Once;

use crate::{
    prelude::*,
    security::securityfs::{SecurityFsDirectory, SecurityFsFile},
};

const NAME_MAX: usize = 255;

#[derive(Debug)]
pub(super) struct SecurityRootNode {
    fields: BranchNodeFields<SecurityDirectory, Self>,
}

#[inherit_methods(from = "self.fields")]
impl SecurityRootNode {
    pub(super) fn singleton() -> &'static Arc<Self> {
        static SINGLETON: Once<Arc<SecurityRootNode>> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(
                SysStr::from("security"),
                SysAttrSet::new_empty(),
                weak_self.clone(),
            ),
        })
    }

    pub(super) fn add_child(&self, new_child: Arc<SecurityDirectory>) -> SysTreeResult<()>;
}

inherit_sys_branch_node!(SecurityRootNode, fields, {
    fn is_root(&self) -> bool {
        true
    }

    fn init_parent(&self, _parent: Weak<dyn SysBranchNode>) {}

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

#[derive(Debug)]
pub(super) struct SecurityDirectory {
    fields: BranchNodeFields<Self, Self>,
    files: &'static [SecurityFsFile],
}

#[inherit_methods(from = "self.fields")]
impl SecurityDirectory {
    fn add_child(&self, new_child: Arc<Self>) -> SysTreeResult<()>;
}

impl SecurityDirectory {
    pub(super) fn new(directory: &SecurityFsDirectory) -> Result<Arc<Self>> {
        let name = directory.name();
        let files = directory.files();
        let directories = directory.directories();
        if !valid_name(name) || (files.is_empty() && directories.is_empty()) {
            return Err(Error::new(Errno::EINVAL));
        }

        let mut attrs = SysAttrSetBuilder::new();
        for (index, file) in files.iter().enumerate() {
            if !valid_name(file.name()) {
                return Err(Error::new(Errno::EINVAL));
            }
            if files[..index]
                .iter()
                .any(|existing| existing.name() == file.name())
            {
                return Err(Error::new(Errno::EEXIST));
            }
            attrs.add(SysStr::from(file.name()), file.perms());
        }
        let attrs = attrs.build()?;

        for (index, child) in directories.iter().enumerate() {
            if !valid_name(child.name()) {
                return Err(Error::new(Errno::EINVAL));
            }
            if files.iter().any(|file| file.name() == child.name())
                || directories[..index]
                    .iter()
                    .any(|existing| existing.name() == child.name())
            {
                return Err(Error::new(Errno::EEXIST));
            }
        }

        let node = Arc::new_cyclic(|weak_self| Self {
            fields: BranchNodeFields::new(SysStr::from(name), attrs, weak_self.clone()),
            files,
        });
        for child in directories {
            node.add_child(Self::new(child)?)?;
        }
        Ok(node)
    }

    fn file(&self, name: &str) -> Option<&SecurityFsFile> {
        self.files.iter().find(|file| file.name() == name)
    }

    pub(super) fn read_file_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> Result<usize> {
        self.file(name)
            .ok_or_else(|| Error::new(Errno::ENOENT))?
            .read_at(offset, writer)
    }

    pub(super) fn write_file_at(
        &self,
        name: &str,
        offset: usize,
        reader: &mut VmReader,
    ) -> Result<usize> {
        self.file(name)
            .ok_or_else(|| Error::new(Errno::ENOENT))?
            .write_at(offset, reader)
    }
}

inherit_sys_branch_node!(SecurityDirectory, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        self.read_file_at(name, offset, writer)
            .map_err(map_callback_error)
    }

    fn write_attr_at(
        &self,
        name: &str,
        offset: usize,
        reader: &mut VmReader,
    ) -> SysTreeResult<usize> {
        self.write_file_at(name, offset, reader)
            .map_err(map_callback_error)
    }
});

fn valid_name(name: &str) -> bool {
    name.len() <= NAME_MAX && aster_systree::is_valid_name(name)
}

fn map_callback_error(error: Error) -> SysTreeError {
    match error.error() {
        Errno::EFAULT => SysTreeError::PageFault,
        Errno::EACCES | Errno::EPERM => SysTreeError::PermissionDenied,
        _ => SysTreeError::AttributeError,
    }
}
