// SPDX-License-Identifier: MPL-2.0

use alloc::str;

use super::{
    file_table::FileDesc,
    inode_handle::InodeHandle,
    path::Dentry,
    rootfs::root_mount,
    utils::{AccessMode, CreationFlags, InodeMode, InodeType, StatusFlags, PATH_MAX, SYMLINKS_MAX},
};
use crate::prelude::*;

/// The file descriptor of the current working directory.
pub const AT_FDCWD: FileDesc = -100;

/// File system resolver.
#[derive(Debug, Clone)]
pub struct FsResolver {
    root: Dentry,
    cwd: Dentry,
}

impl FsResolver {
    /// Creates a new file system resolver.
    pub fn new() -> Self {
        Self {
            root: Dentry::new_fs_root(root_mount().clone()),
            cwd: Dentry::new_fs_root(root_mount().clone()),
        }
    }

    /// Gets the root directory.
    pub fn root(&self) -> &Dentry {
        &self.root
    }

    /// Gets the current working directory.
    pub fn cwd(&self) -> &Dentry {
        &self.cwd
    }

    /// Sets the current working directory to the given `dentry`.
    pub fn set_cwd(&mut self, dentry: Dentry) {
        self.cwd = dentry;
    }

    /// Sets the root directory to the given `dentry`.
    pub fn set_root(&mut self, dentry: Dentry) {
        self.root = dentry;
    }

    /// Opens or creates a file inode handler.
    pub fn open(&self, path: &FsPath, flags: u32, mode: u16) -> Result<InodeHandle> {
        let open_args = OpenArgs::from_flags_and_mode(flags, mode)?;

        let follow_tail_link = open_args.follow_tail_link();
        let stop_on_parent = false;
        let mut lookup_ctx = LookupCtx::new(follow_tail_link, stop_on_parent);

        let lookup_res = self.lookup_inner(path, &mut lookup_ctx);

        let inode_handle = match lookup_res {
            Ok(target_dentry) => self.open_existing_file(target_dentry, &open_args)?,
            Err(e)
                if e.error() == Errno::ENOENT
                    && open_args.creation_flags.contains(CreationFlags::O_CREAT) =>
            {
                self.create_new_file(&open_args, &mut lookup_ctx)?
            }
            Err(e) => return Err(e),
        };

        Ok(inode_handle)
    }

    fn open_existing_file(
        &self,
        target_dentry: Dentry,
        open_args: &OpenArgs,
    ) -> Result<InodeHandle> {
        let inode = target_dentry.inode();
        let inode_type = inode.type_();
        let creation_flags = &open_args.creation_flags;

        match inode_type {
            InodeType::NamedPipe => {
                warn!("NamedPipe doesn't support additional operation when opening.");
                debug!("Open NamedPipe with args: {open_args:?}.");
            }
            InodeType::SymLink => {
                if creation_flags.contains(CreationFlags::O_NOFOLLOW)
                    && !open_args.status_flags.contains(StatusFlags::O_PATH)
                {
                    return_errno_with_message!(Errno::ELOOP, "file is a symlink");
                }
            }
            _ => {}
        }

        if creation_flags.contains(CreationFlags::O_CREAT)
            && creation_flags.contains(CreationFlags::O_EXCL)
        {
            return_errno_with_message!(Errno::EEXIST, "file exists");
        }
        if creation_flags.contains(CreationFlags::O_DIRECTORY) && inode_type != InodeType::Dir {
            return_errno_with_message!(
                Errno::ENOTDIR,
                "O_DIRECTORY is specified but file is not a directory"
            );
        }

        if creation_flags.contains(CreationFlags::O_TRUNC) {
            target_dentry.resize(0)?;
        }
        InodeHandle::new(target_dentry, open_args.access_mode, open_args.status_flags)
    }

    fn create_new_file(
        &self,
        open_args: &OpenArgs,
        lookup_ctx: &mut LookupCtx,
    ) -> Result<InodeHandle> {
        if open_args
            .creation_flags
            .contains(CreationFlags::O_DIRECTORY)
        {
            return_errno_with_message!(Errno::ENOTDIR, "cannot create directory");
        }
        if lookup_ctx.tail_is_dir() {
            return_errno_with_message!(Errno::EISDIR, "path refers to a directory");
        }

        let parent = lookup_ctx
            .parent()
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "parent not found"))?;
        if !parent.mode()?.is_writable() {
            return_errno_with_message!(Errno::EACCES, "file cannot be created");
        }

        let tail_file_name = lookup_ctx.tail_file_name().unwrap();
        let new_dentry =
            parent.new_fs_child(&tail_file_name, InodeType::File, open_args.inode_mode)?;
        // Don't check access mode for newly created file
        InodeHandle::new_unchecked_access(new_dentry, open_args.access_mode, open_args.status_flags)
    }

    /// Lookups the target dentry according to the `path`.
    /// Symlinks are always followed.
    pub fn lookup(&self, path: &FsPath) -> Result<Dentry> {
        let (follow_tail_link, stop_on_parent) = (true, false);
        self.lookup_inner(path, &mut LookupCtx::new(follow_tail_link, stop_on_parent))
    }

    /// Lookups the target dentry according to the `path`.
    /// If the last component is a symlink, it will not be followed.
    pub fn lookup_no_follow(&self, path: &FsPath) -> Result<Dentry> {
        let (follow_tail_link, stop_on_parent) = (false, false);
        self.lookup_inner(path, &mut LookupCtx::new(follow_tail_link, stop_on_parent))
    }

    fn lookup_inner(&self, path: &FsPath, lookup_ctx: &mut LookupCtx) -> Result<Dentry> {
        let dentry = match path.inner {
            FsPathInner::Absolute(path) => {
                self.lookup_from_parent(&self.root, path.trim_start_matches('/'), lookup_ctx)?
            }
            FsPathInner::CwdRelative(path) => {
                self.lookup_from_parent(&self.cwd, path, lookup_ctx)?
            }
            FsPathInner::Cwd => self.cwd.clone(),
            FsPathInner::FdRelative(fd, path) => {
                let parent = self.lookup_from_fd(fd)?;
                self.lookup_from_parent(&parent, path, lookup_ctx)?
            }
            FsPathInner::Fd(fd) => self.lookup_from_fd(fd)?,
        };

        Ok(dentry)
    }

    /// Lookups the target dentry according to the parent directory dentry.
    ///
    /// The length of `path` cannot exceed `PATH_MAX`.
    /// If `path` ends with `/`, then the returned inode must be a directory inode.
    ///
    /// While looking up the dentry, symbolic links will be followed for
    /// at most `SYMLINKS_MAX` times.
    ///
    /// If `follow_tail_link` is true and the trailing component is a symlink,
    /// it will be followed.
    /// Symlinks in earlier components of the path will always be followed.
    #[allow(clippy::redundant_closure)]
    fn lookup_from_parent(
        &self,
        parent: &Dentry,
        relative_path: &str,
        lookup_ctx: &mut LookupCtx,
    ) -> Result<Dentry> {
        debug_assert!(!relative_path.starts_with('/'));

        if relative_path.len() > PATH_MAX {
            return_errno_with_message!(Errno::ENAMETOOLONG, "path is too long");
        }
        if relative_path.is_empty() {
            return Ok(parent.clone());
        }

        // To handle symlinks
        let follow_tail_link = lookup_ctx.follow_tail_link;
        let mut link_path_opt = None;
        let mut follows = 0;

        // Initialize the first dentry and the relative path
        let (mut dentry, mut relative_path) = (parent.clone(), relative_path);

        while !relative_path.is_empty() {
            let (next_name, path_remain, must_be_dir) =
                if let Some((prefix, suffix)) = relative_path.split_once('/') {
                    let suffix = suffix.trim_start_matches('/');
                    (prefix, suffix, true)
                } else {
                    (relative_path, "", false)
                };

            // Iterate next dentry
            let next_is_tail = path_remain.is_empty();
            if next_is_tail && lookup_ctx.stop_on_parent {
                lookup_ctx.set_tail_file(next_name, must_be_dir);
                return Ok(dentry);
            }

            let next_dentry = match dentry.lookup(next_name) {
                Ok(dentry) => dentry,
                Err(e) => {
                    if next_is_tail && e.error() == Errno::ENOENT && lookup_ctx.tail_file.is_none()
                    {
                        lookup_ctx.set_tail_file(next_name, must_be_dir);
                        lookup_ctx.set_parent(&dentry);
                    }
                    return Err(e);
                }
            };
            let next_type = next_dentry.type_();

            // If next inode is a symlink, follow symlinks at most `SYMLINKS_MAX` times.
            if next_type == InodeType::SymLink && (follow_tail_link || !next_is_tail) {
                if follows >= SYMLINKS_MAX {
                    return_errno_with_message!(Errno::ELOOP, "too many symlinks");
                }
                let link_path_remain = {
                    let mut tmp_link_path = next_dentry.inode().read_link()?;
                    if tmp_link_path.is_empty() {
                        return_errno_with_message!(Errno::ENOENT, "empty symlink");
                    }
                    if !path_remain.is_empty() {
                        tmp_link_path += "/";
                        tmp_link_path += path_remain;
                    } else if must_be_dir {
                        tmp_link_path += "/";
                    }
                    tmp_link_path
                };

                // Change the dentry and relative path according to symlink
                if link_path_remain.starts_with('/') {
                    dentry = self.root.clone();
                }
                let link_path = link_path_opt.get_or_insert_with(|| String::new());
                link_path.clear();
                link_path.push_str(link_path_remain.trim_start_matches('/'));
                relative_path = link_path;
                follows += 1;
            } else {
                // If path ends with `/`, the inode must be a directory
                if must_be_dir && next_type != InodeType::Dir {
                    return_errno_with_message!(Errno::ENOTDIR, "inode is not dir");
                }
                dentry = next_dentry;
                relative_path = path_remain;
            }
        }

        Ok(dentry)
    }

    /// Lookups the target dentry according to the given `fd`.
    pub fn lookup_from_fd(&self, fd: FileDesc) -> Result<Dentry> {
        let current = current!();
        let file_table = current.file_table().lock();
        let inode_handle = file_table
            .get_file(fd)?
            .downcast_ref::<InodeHandle>()
            .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
        Ok(inode_handle.dentry().clone())
    }

    /// Lookups the target parent directory dentry and
    /// the base file name according to the given `path`.
    ///
    /// If the last component is a symlink, do not deference it.
    pub fn lookup_dir_and_base_name(&self, path: &FsPath) -> Result<(Dentry, String)> {
        if matches!(path.inner, FsPathInner::Fd(_)) {
            return_errno!(Errno::ENOENT);
        }

        let (follow_tail_link, stop_on_parent) = (false, true);
        let mut lookup_ctx = LookupCtx::new(follow_tail_link, stop_on_parent);
        let parent_dir = self.lookup_inner(path, &mut lookup_ctx)?;

        let tail_file_name = lookup_ctx.tail_file_name().unwrap();
        Ok((parent_dir, tail_file_name))
    }

    /// Lookups the target parent directory dentry and checks whether
    /// the base file does not exist yet according to the given `path`.
    ///
    /// `is_dir` is used to determine whether a directory needs to be created.
    ///
    /// # Usage case
    ///
    /// `mkdir`, `mknod`, `link`, and `symlink` all need to create
    /// new file and all need to perform unique processing on the last
    /// component of the path name. It is used to provide a unified
    /// method for pathname lookup and error handling.
    pub fn lookup_dir_and_new_basename(
        &self,
        path: &FsPath,
        is_dir: bool,
    ) -> Result<(Dentry, String)> {
        if matches!(path.inner, FsPathInner::Fd(_)) {
            return_errno!(Errno::ENOENT);
        }

        let (follow_tail_link, stop_on_parent) = (false, true);
        let mut lookup_ctx = LookupCtx::new(follow_tail_link, stop_on_parent);
        let parent_dir = self.lookup_inner(path, &mut lookup_ctx)?;
        let tail_file_name = lookup_ctx.tail_file_name().ok_or_else(|| {
            // If the path is the root directory ("/"), there is no basename,
            // so this operation is not allowed.
            Error::with_message(Errno::EEXIST, "operation not allowed on root directory")
        })?;

        if parent_dir
            .lookup(tail_file_name.trim_end_matches('/'))
            .is_ok()
        {
            return_errno_with_message!(Errno::EEXIST, "file exists");
        }
        if !is_dir && lookup_ctx.tail_is_dir() {
            return_errno_with_message!(Errno::ENOENT, "No such file or directory");
        }

        Ok((parent_dir.clone(), tail_file_name))
    }
}

impl Default for FsResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Context information describing one lookup operation.
#[derive(Debug)]
struct LookupCtx {
    follow_tail_link: bool,
    stop_on_parent: bool,
    // (file_name, file_is_dir)
    tail_file: Option<(String, bool)>,
    parent: Option<Dentry>,
}

impl LookupCtx {
    pub fn new(follow_tail_link: bool, stop_on_parent: bool) -> Self {
        Self {
            follow_tail_link,
            stop_on_parent,
            tail_file: None,
            parent: None,
        }
    }

    pub fn tail_file_name(&self) -> Option<String> {
        self.tail_file.as_ref().map(|(file_name, file_is_dir)| {
            let mut tail_file_name = file_name.clone();
            if *file_is_dir {
                tail_file_name += "/";
            }
            tail_file_name
        })
    }

    pub fn tail_is_dir(&self) -> bool {
        self.tail_file
            .as_ref()
            .map(|(_, is_dir)| *is_dir)
            .unwrap_or(false)
    }

    pub fn parent(&self) -> Option<&Dentry> {
        self.parent.as_ref()
    }

    pub fn set_tail_file(&mut self, file_name: &str, file_is_dir: bool) {
        let _ = self.tail_file.insert((file_name.to_string(), file_is_dir));
    }

    pub fn set_parent(&mut self, parent: &Dentry) {
        let _ = self.parent.insert(parent.clone());
    }
}

#[derive(Debug)]
/// Arguments for an open request.
struct OpenArgs {
    creation_flags: CreationFlags,
    status_flags: StatusFlags,
    access_mode: AccessMode,
    inode_mode: InodeMode,
}

impl OpenArgs {
    pub fn from_flags_and_mode(flags: u32, mode: u16) -> Result<Self> {
        let creation_flags = CreationFlags::from_bits_truncate(flags);
        let status_flags = StatusFlags::from_bits_truncate(flags);
        let access_mode = AccessMode::from_u32(flags)?;
        let inode_mode = InodeMode::from_bits_truncate(mode);
        Ok(Self {
            creation_flags,
            status_flags,
            access_mode,
            inode_mode,
        })
    }

    pub fn follow_tail_link(&self) -> bool {
        !(self.creation_flags.contains(CreationFlags::O_NOFOLLOW)
            || self.creation_flags.contains(CreationFlags::O_CREAT)
                && self.creation_flags.contains(CreationFlags::O_EXCL))
    }
}

/// Path in the file system.
#[derive(Debug)]
pub struct FsPath<'a> {
    inner: FsPathInner<'a>,
}

#[derive(Debug)]
enum FsPathInner<'a> {
    // Absolute path
    Absolute(&'a str),
    // Path is relative to `Cwd`
    CwdRelative(&'a str),
    // Current working directory
    Cwd,
    // Path is relative to the directory fd
    FdRelative(FileDesc, &'a str),
    // Fd
    Fd(FileDesc),
}

impl<'a> FsPath<'a> {
    /// Creates a new `FsPath` from the given `dirfd` and `path`.
    pub fn new(dirfd: FileDesc, path: &'a str) -> Result<Self> {
        if path.len() > PATH_MAX {
            return_errno_with_message!(Errno::ENAMETOOLONG, "path name too long");
        }

        let fs_path_inner = if path.starts_with('/') {
            FsPathInner::Absolute(path)
        } else if dirfd >= 0 {
            if path.is_empty() {
                FsPathInner::Fd(dirfd)
            } else {
                FsPathInner::FdRelative(dirfd, path)
            }
        } else if dirfd == AT_FDCWD {
            if path.is_empty() {
                FsPathInner::Cwd
            } else {
                FsPathInner::CwdRelative(path)
            }
        } else {
            return_errno_with_message!(Errno::EBADF, "invalid dirfd number");
        };

        Ok(Self {
            inner: fs_path_inner,
        })
    }
}

impl<'a> TryFrom<&'a str> for FsPath<'a> {
    type Error = crate::error::Error;

    fn try_from(path: &'a str) -> Result<FsPath<'a>> {
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is an empty string");
        }
        FsPath::new(AT_FDCWD, path)
    }
}

/// Splits a `path` to (`dir_path`, `file_name`).
///
/// The `dir_path` must be a directory.
///
/// The `file_name` is the last component. It can be suffixed by "/".
///
/// Example:
///
/// The path "/dir/file/" will be split to ("/dir", "file/").
pub fn split_path(path: &str) -> (&str, &str) {
    let file_name = path
        .split_inclusive('/')
        .filter(|&x| x != "/")
        .last()
        .unwrap_or(".");

    let mut split = path.trim_end_matches('/').rsplitn(2, '/');
    let dir_path = if split.next().unwrap().is_empty() {
        "/"
    } else {
        let mut dir = split.next().unwrap_or(".").trim_end_matches('/');
        if dir.is_empty() {
            dir = "/";
        }
        dir
    };

    (dir_path, file_name)
}
