// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use inherit_methods_macro::inherit_methods;

use crate::{
    fs::{
        inode_handle::{FileIo, InodeHandle},
        pipe::Pipe,
        pseudofs::{PipeFs, PseudoInode, PseudoInodeType},
        utils::{
            AccessMode, Extension, FileSystem, Inode, InodeIo, InodeMode, InodeType, Metadata,
            StatusFlags, mkmod,
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

/// Creates a pair of connected pipe file handles with the default capacity.
pub fn new_file_pair() -> Result<(Arc<InodeHandle>, Arc<InodeHandle>)> {
    let pipe_inode = Arc::new(AnonPipeInode::new());
    let path = PipeFs::new_path(pipe_inode);

    let reader = InodeHandle::new_unchecked_access(
        path.clone(),
        AccessMode::O_RDONLY,
        StatusFlags::empty(),
    )?;
    let writer =
        InodeHandle::new_unchecked_access(path, AccessMode::O_WRONLY, StatusFlags::empty())?;

    Ok((Arc::new(reader), Arc::new(writer)))
}

/// An anonymous pipe inode.
pub struct AnonPipeInode {
    /// The underlying pipe backend.
    pipe: Pipe,
    pseudo_inode: PseudoInode,
}

impl AnonPipeInode {
    fn new() -> Self {
        let pipe = Pipe::new();

        let pseudo_inode = PipeFs::singleton().alloc_inode(
            PseudoInodeType::Pipe,
            mkmod!(u+rw),
            Uid::new_root(),
            Gid::new_root(),
        );

        Self { pipe, pseudo_inode }
    }
}

#[inherit_methods(from = "self.pseudo_inode")]
impl InodeIo for AnonPipeInode {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status: StatusFlags,
    ) -> Result<usize>;
    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status: StatusFlags,
    ) -> Result<usize>;
}

#[inherit_methods(from = "self.pseudo_inode")]
impl Inode for AnonPipeInode {
    fn size(&self) -> usize;
    fn resize(&self, _new_size: usize) -> Result<()>;
    fn metadata(&self) -> Metadata;
    fn extension(&self) -> &Extension;
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

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        Some(self.pipe.open_anon(access_mode, status_flags))
    }
}
