// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs,
    fs::{
        file_handle::FileLike,
        file_table::{FdFlags, FileDesc},
        fs_resolver::{AT_FDCWD, FsPath, FsResolver, LookupResult, PathOrInode},
        inode_handle::InodeHandle,
        pipe::{AnonPipeFile, AnonPipeInode},
        ramfs::memfd::{MemfdFile, MemfdInode},
        utils::{AccessMode, CreationFlags, InodeMode, InodeType, OpenArgs, StatusFlags},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_openat(
    dirfd: FileDesc,
    path_addr: Vaddr,
    flags: u32,
    mode: u16,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, path = {:?}, flags = {}, mode = {}",
        dirfd, path, flags, mode
    );

    let file_handle = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(dirfd, path.as_ref())?;

        let fs_ref = ctx.thread_local.borrow_fs();
        let mask_mode = mode & !fs_ref.umask().get();

        let fs_resolver = fs_ref.resolver().read();
        do_open(
            &fs_resolver,
            &fs_path,
            flags,
            InodeMode::from_bits_truncate(mask_mode),
        )
        .map_err(|err| match err.error() {
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?
    };

    let fd = {
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.unwrap().write();
        let fd_flags =
            if CreationFlags::from_bits_truncate(flags).contains(CreationFlags::O_CLOEXEC) {
                FdFlags::CLOEXEC
            } else {
                FdFlags::empty()
            };
        file_table_locked.insert(file_handle.clone(), fd_flags)
    };
    let file_like: Arc<dyn FileLike> = file_handle;
    fs::notify::on_open(&file_like);
    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_open(path_addr: Vaddr, flags: u32, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_openat(AT_FDCWD, path_addr, flags, mode, ctx)
}

pub fn sys_creat(path_addr: Vaddr, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    let flags =
        AccessMode::O_WRONLY as u32 | CreationFlags::O_CREAT.bits() | CreationFlags::O_TRUNC.bits();
    self::sys_openat(AT_FDCWD, path_addr, flags, mode, ctx)
}

fn do_open(
    fs_resolver: &FsResolver,
    fs_path: &FsPath,
    flags: u32,
    mode: InodeMode,
) -> Result<Arc<dyn FileLike>> {
    let open_args = OpenArgs::from_flags_and_mode(flags, mode)?;

    let lookup_res = if open_args.follow_tail_link() {
        fs_resolver.lookup_unresolved(fs_path)?
    } else {
        fs_resolver.lookup_unresolved_no_follow(fs_path)?
    };

    let file_handle: Arc<dyn FileLike> = match lookup_res {
        LookupResult::Resolved(target) => match target {
            PathOrInode::Path(path) => Arc::new(path.open(open_args)?),
            PathOrInode::Inode(inode) => {
                if let Ok(memfd_inode) = Arc::downcast::<MemfdInode>(inode.clone()) {
                    Arc::new(MemfdFile::open(memfd_inode, open_args)?)
                } else if let Ok(pipe_inode) = Arc::downcast::<AnonPipeInode>(inode) {
                    Arc::new(AnonPipeFile::open(
                        pipe_inode,
                        open_args.access_mode,
                        open_args.status_flags,
                    )?)
                } else {
                    return_errno_with_message!(Errno::ENXIO, "the inode is not re-openable")
                }
            }
        },
        LookupResult::AtParent(result) => {
            if !open_args.creation_flags.contains(CreationFlags::O_CREAT)
                || open_args.status_flags.contains(StatusFlags::O_PATH)
            {
                return_errno_with_message!(Errno::ENOENT, "the file does not exist");
            }
            if open_args
                .creation_flags
                .contains(CreationFlags::O_DIRECTORY)
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "O_CREAT and O_DIRECTORY cannot be specified together"
                );
            }
            if result.target_is_dir() {
                return_errno_with_message!(
                    Errno::EISDIR,
                    "O_CREAT is specified but the file is a directory"
                );
            }

            let (parent, tail_name) = result.into_parent_and_basename();
            let new_path =
                parent.new_fs_child(&tail_name, InodeType::File, open_args.inode_mode)?;
            fs::notify::on_create(&parent, || tail_name.clone());

            // Don't check access mode for newly created file.
            Arc::new(InodeHandle::new_unchecked_access(
                new_path,
                open_args.access_mode,
                open_args.status_flags,
            )?)
        }
    };

    Ok(file_handle)
}
