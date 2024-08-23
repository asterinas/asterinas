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

#[derive(Debug)]
pub struct FsResolver {
    root: Arc<Dentry>,
    cwd: Arc<Dentry>,
}

impl Clone for FsResolver {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            cwd: self.cwd.clone(),
        }
    }
}

impl Default for FsResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl FsResolver {
    pub fn new() -> Self {
        Self {
            root: Dentry::new_fs_root(root_mount().clone()),
            cwd: Dentry::new_fs_root(root_mount().clone()),
        }
    }

    /// Get the root directory
    pub fn root(&self) -> &Arc<Dentry> {
        &self.root
    }

    /// Get the current working directory
    pub fn cwd(&self) -> &Arc<Dentry> {
        &self.cwd
    }

    /// Set the current working directory.
    pub fn set_cwd(&mut self, dentry: Arc<Dentry>) {
        self.cwd = dentry;
    }

    /// Set the root directory
    pub fn set_root(&mut self, dentry: Arc<Dentry>) {
        self.root = dentry;
    }

    /// Open or create a file inode handler.
    pub fn open(&self, path: &FsPath, flags: u32, mode: u16) -> Result<InodeHandle> {
        let creation_flags = CreationFlags::from_bits_truncate(flags);
        let status_flags = StatusFlags::from_bits_truncate(flags);
        let access_mode = AccessMode::from_u32(flags)?;
        let inode_mode = InodeMode::from_bits_truncate(mode);

        let follow_tail_link = !(creation_flags.contains(CreationFlags::O_NOFOLLOW)
            || creation_flags.contains(CreationFlags::O_CREAT)
                && creation_flags.contains(CreationFlags::O_EXCL));
        let inode_handle = match self.lookup_inner(path, follow_tail_link) {
            Ok(dentry) => {
                let inode = dentry.inode();
                match inode.type_() {
                    InodeType::NamedPipe => {
                        warn!("NamedPipe doesn't support additional operation when opening.");
                        debug!("Open NamedPipe. creation_flags:{:?}, status_flags:{:?}, access_mode:{:?}, inode_mode:{:?}",creation_flags,status_flags,access_mode,inode_mode);
                    }
                    InodeType::SymLink => {
                        if creation_flags.contains(CreationFlags::O_NOFOLLOW)
                            && !status_flags.contains(StatusFlags::O_PATH)
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
                if creation_flags.contains(CreationFlags::O_DIRECTORY)
                    && inode.type_() != InodeType::Dir
                {
                    return_errno_with_message!(
                        Errno::ENOTDIR,
                        "O_DIRECTORY is specified but file is not a directory"
                    );
                }

                if creation_flags.contains(CreationFlags::O_TRUNC) {
                    dentry.resize(0)?;
                }
                InodeHandle::new(dentry, access_mode, status_flags)?
            }
            Err(e)
                if e.error() == Errno::ENOENT
                    && creation_flags.contains(CreationFlags::O_CREAT) =>
            {
                if creation_flags.contains(CreationFlags::O_DIRECTORY) {
                    return_errno_with_message!(Errno::ENOTDIR, "cannot create directory");
                }
                let (dir_dentry, file_name) =
                    self.lookup_dir_and_base_name_inner(path, follow_tail_link)?;
                if file_name.ends_with('/') {
                    return_errno_with_message!(Errno::EISDIR, "path refers to a directory");
                }
                if !dir_dentry.mode()?.is_writable() {
                    return_errno_with_message!(Errno::EACCES, "file cannot be created");
                }

                let dentry = dir_dentry.new_fs_child(&file_name, InodeType::File, inode_mode)?;
                // Don't check access mode for newly created file
                InodeHandle::new_unchecked_access(dentry, access_mode, status_flags)?
            }
            Err(e) => return Err(e),
        };

        Ok(inode_handle)
    }

    /// Lookup dentry according to FsPath, always follow symlinks
    pub fn lookup(&self, path: &FsPath) -> Result<Arc<Dentry>> {
        self.lookup_inner(path, true)
    }

    /// Lookup dentry according to FsPath, do not follow it if last component is a symlink
    pub fn lookup_no_follow(&self, path: &FsPath) -> Result<Arc<Dentry>> {
        self.lookup_inner(path, false)
    }

    fn lookup_inner(&self, path: &FsPath, follow_tail_link: bool) -> Result<Arc<Dentry>> {
        let dentry = match path.inner {
            FsPathInner::Absolute(path) => {
                self.lookup_from_parent(&self.root, path.trim_start_matches('/'), follow_tail_link)?
            }
            FsPathInner::CwdRelative(path) => {
                self.lookup_from_parent(&self.cwd, path, follow_tail_link)?
            }
            FsPathInner::Cwd => self.cwd.clone(),
            FsPathInner::FdRelative(fd, path) => {
                let parent = self.lookup_from_fd(fd)?;
                self.lookup_from_parent(&parent, path, follow_tail_link)?
            }
            FsPathInner::Fd(fd) => self.lookup_from_fd(fd)?,
        };

        Ok(dentry)
    }

    /// Lookup dentry from parent
    ///
    /// The length of `path` cannot exceed PATH_MAX.
    /// If `path` ends with `/`, then the returned inode must be a directory inode.
    ///
    /// While looking up the dentry, symbolic links will be followed for
    /// at most `SYMLINKS_MAX` times.
    ///
    /// If `follow_tail_link` is true and the trailing component is a symlink,
    /// it will be followed.
    /// Symlinks in earlier components of the path will always be followed.
    fn lookup_from_parent(
        &self,
        parent: &Arc<Dentry>,
        relative_path: &str,
        follow_tail_link: bool,
    ) -> Result<Arc<Dentry>> {
        debug_assert!(!relative_path.starts_with('/'));

        if relative_path.len() > PATH_MAX {
            return_errno_with_message!(Errno::ENAMETOOLONG, "path is too long");
        }

        // To handle symlinks
        let mut link_path = String::new();
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
            let next_dentry = dentry.lookup(next_name)?;
            let next_type = next_dentry.type_();
            let next_is_tail = path_remain.is_empty();

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
                link_path.clear();
                link_path.push_str(link_path_remain.trim_start_matches('/'));
                relative_path = &link_path;
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

    /// Lookup dentry from the giving fd
    pub fn lookup_from_fd(&self, fd: FileDesc) -> Result<Arc<Dentry>> {
        let current = current!();
        let file_table = current.file_table().lock();
        let inode_handle = file_table
            .get_file(fd)?
            .downcast_ref::<InodeHandle>()
            .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
        Ok(inode_handle.dentry().clone())
    }

    /// Lookup the dir dentry and base file name of the giving path.
    ///
    /// If the last component is a symlink, do not deference it
    pub fn lookup_dir_and_base_name(&self, path: &FsPath) -> Result<(Arc<Dentry>, String)> {
        self.lookup_dir_and_base_name_inner(path, false)
    }

    fn lookup_dir_and_base_name_inner(
        &self,
        path: &FsPath,
        follow_tail_link: bool,
    ) -> Result<(Arc<Dentry>, String)> {
        let (mut dir_dentry, mut base_name) = match path.inner {
            FsPathInner::Absolute(path) => {
                let (dir, file_name) = split_path(path);
                (
                    self.lookup_from_parent(&self.root, dir.trim_start_matches('/'), true)?,
                    String::from(file_name),
                )
            }
            FsPathInner::CwdRelative(path) => {
                let (dir, file_name) = split_path(path);
                (
                    self.lookup_from_parent(&self.cwd, dir, true)?,
                    String::from(file_name),
                )
            }
            FsPathInner::FdRelative(fd, path) => {
                let (dir, file_name) = split_path(path);
                let parent = self.lookup_from_fd(fd)?;
                (
                    self.lookup_from_parent(&parent, dir, true)?,
                    String::from(file_name),
                )
            }
            _ => return_errno!(Errno::ENOENT),
        };
        if !follow_tail_link {
            return Ok((dir_dentry, base_name));
        }

        // Dereference the tail symlinks if needed
        loop {
            match dir_dentry.lookup(base_name.trim_end_matches('/')) {
                Ok(dentry) if dentry.type_() == InodeType::SymLink => {
                    let link = {
                        let mut link = dentry.inode().read_link()?;
                        if link.is_empty() {
                            return_errno_with_message!(Errno::ENOENT, "invalid symlink");
                        }
                        if base_name.ends_with('/') && !link.ends_with('/') {
                            link += "/";
                        }
                        link
                    };
                    let (dir, file_name) = split_path(&link);
                    if dir.starts_with('/') {
                        dir_dentry =
                            self.lookup_from_parent(&self.root, dir.trim_start_matches('/'), true)?;
                        base_name = String::from(file_name);
                    } else {
                        dir_dentry = self.lookup_from_parent(&dir_dentry, dir, true)?;
                        base_name = String::from(file_name);
                    }
                }
                _ => break,
            }
        }

        Ok((dir_dentry, base_name))
    }

    /// The method used to create a new file using pathname.
    ///
    /// For example, mkdir, mknod, link, and symlink all need to create
    /// new file and all need to perform unique processing on the last
    /// component of the path name. It is used to provide a unified
    /// method for pathname lookup and error handling.
    ///
    /// is_dir is used to determine whether a directory needs to be created.
    pub fn lookup_dir_and_new_basename(
        &self,
        path: &FsPath,
        is_dir: bool,
    ) -> Result<(Arc<Dentry>, String)> {
        let (dir_dentry, filename) = self.lookup_dir_and_base_name_inner(path, false)?;
        if dir_dentry.lookup(filename.trim_end_matches('/')).is_ok() {
            return_errno_with_message!(Errno::EEXIST, "file exists");
        }

        if !is_dir && filename.ends_with('/') {
            return_errno_with_message!(Errno::ENOENT, "No such file or directory");
        }

        Ok((dir_dentry, filename))
    }
}

pub const AT_FDCWD: FileDesc = -100;

#[derive(Debug)]
pub struct FsPath<'a> {
    inner: FsPathInner<'a>,
}

#[derive(Debug)]
enum FsPathInner<'a> {
    // absolute path
    Absolute(&'a str),
    // path is relative to Cwd
    CwdRelative(&'a str),
    // Cwd
    Cwd,
    // path is relative to DirFd
    FdRelative(FileDesc, &'a str),
    // Fd
    Fd(FileDesc),
}

impl<'a> FsPath<'a> {
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

    fn try_from(path: &'a str) -> Result<FsPath> {
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is an empty string");
        }
        FsPath::new(AT_FDCWD, path)
    }
}

/// Split a `path` to (`dir_path`, `file_name`).
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
