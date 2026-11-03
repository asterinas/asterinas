// SPDX-License-Identifier: MPL-2.0

//! Immutable securityfs directory and file-node model.
//!
//! [`SecurityRootNode`] contains top-level [`SecurityDirectory`] instances.
//! Each directory maps the static file declarations from the public securityfs
//! seam into SysTree attributes consumed by the inode adapter.

use aster_systree::{
    BranchNodeFields, Error as SysTreeError, NormalNodeFields, Result as SysTreeResult, SysAttrSet,
    SysAttrSetBuilder, SysBranchNode, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
    inherit_sys_leaf_node,
};
use inherit_methods_macro::inherit_methods;
use spin::Once;

use crate::{prelude::*, security::securityfs::SecurityFsFile};

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
    fields: NormalNodeFields<Self>,
    files: &'static [SecurityFsFile],
}

impl SecurityDirectory {
    pub(super) fn new(name: &'static str, files: &'static [SecurityFsFile]) -> Result<Arc<Self>> {
        if !valid_name(name) || files.is_empty() {
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

        Ok(Arc::new_cyclic(|weak_self| Self {
            fields: NormalNodeFields::new(SysStr::from(name), attrs, weak_self.clone()),
            files,
        }))
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

inherit_sys_leaf_node!(SecurityDirectory, fields, {
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
