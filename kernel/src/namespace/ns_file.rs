// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use inherit_methods_macro::inherit_methods;

use crate::{
    fs::utils::{FileSystem, Inode, InodeMode, InodeType, Metadata},
    namespace::NameSpace,
    prelude::*,
    process::{Gid, Uid},
};

/// Represents an open file handle to a specific namespace.
///
/// The `NsFile` correspond to the special files found in
/// `/proc/[pid]/ns/`, such as `/proc/self/ns/user`.
pub struct NsFile(Arc<dyn NameSpace>);

impl NsFile {
    pub const fn new(ns: Arc<dyn NameSpace>) -> Self {
        Self(ns)
    }

    pub fn ns(&self) -> &Arc<dyn NameSpace> {
        &self.0
    }
}

#[inherit_methods(from = "self.0.inode()")]
impl Inode for NsFile {
    fn size(&self) -> usize;
    fn resize(&self, new_size: usize) -> Result<()>;
    fn metadata(&self) -> Metadata;
    fn ino(&self) -> u64;
    fn type_(&self) -> InodeType;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;
    fn atime(&self) -> Duration;
    fn set_atime(&self, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, time: Duration);
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, time: Duration);
    fn fs(&self) -> Arc<dyn FileSystem>;
}
