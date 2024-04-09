// SPDX-License-Identifier: MPL-2.0

use alloc::str;

use super::{
    file_table::FileDesc,
    inode_handle::InodeHandle,
    rootfs::root_mount,
    utils::{
        AccessMode, CreationFlags, InodeMode, InodeType, Path, StatusFlags, PATH_MAX, SYMLINKS_MAX,
    },
};
use crate::prelude::*;

#[derive(Debug)]
pub struct FsResolver {
    root: Arc<Path>,
    cwd: Arc<Path>,
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
            root: Path::new(root_mount().clone(), root_mount().root_dentry().clone()),
            cwd: Path::new(root_mount().clone(), root_mount().root_dentry().clone()),
        }
    }

    /// Get the root directory
    pub fn root(&self) -> &Arc<Path> {
        &self.root
    }

    /// Get the current working directory
    pub fn cwd(&self) -> &Arc<Path> {
        &self.cwd
    }

    /// Set the current working directory.
    pub fn set_cwd(&mut self, path: Arc<Path>) {
        self.cwd = path;
    }

    /// Set the root directory
    pub fn set_root(&mut self, path: Arc<Path>) {
        self.root = path;
    }

    /// Open or create a file inode handler.
    pub fn open(&self, pathname: &FsPath, flags: u32, mode: u16) -> Result<InodeHandle> {
        let creation_flags = CreationFlags::from_bits_truncate(flags);
        let status_flags = StatusFlags::from_bits_truncate(flags);
        let access_mode = AccessMode::from_u32(flags)?;
        let inode_mode = InodeMode::from_bits_truncate(mode);

        let follow_tail_link = !(creation_flags.contains(CreationFlags::O_NOFOLLOW)
            || creation_flags.contains(CreationFlags::O_CREAT)
                && creation_flags.contains(CreationFlags::O_EXCL));
        let path = match self.lookup_inner(pathname, follow_tail_link) {
            Ok(path) => {
                let inode = path.dentry().inode();
                if inode.type_() == InodeType::SymLink
                    && creation_flags.contains(CreationFlags::O_NOFOLLOW)
                    && !status_flags.contains(StatusFlags::O_PATH)
                {
                    return_errno_with_message!(Errno::ELOOP, "file is a symlink");
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
                path
            }
            Err(e)
                if e.error() == Errno::ENOENT
                    && creation_flags.contains(CreationFlags::O_CREAT) =>
            {
                if creation_flags.contains(CreationFlags::O_DIRECTORY) {
                    return_errno_with_message!(Errno::ENOTDIR, "cannot create directory");
                }
                let (dir_path, file_name) =
                    self.lookup_dir_and_base_name_inner(pathname, follow_tail_link)?;
                if file_name.ends_with('/') {
                    return_errno_with_message!(Errno::EISDIR, "path refers to a directory");
                }
                if !dir_path.dentry().mode()?.is_writable() {
                    return_errno_with_message!(Errno::EACCES, "file cannot be created");
                }
                let dir_dentry =
                    dir_path
                        .dentry()
                        .create(&file_name, InodeType::File, inode_mode)?;
                Path::new(dir_path.mount_node().clone(), dir_dentry.clone())
            }
            Err(e) => return Err(e),
        };

        let inode_handle = InodeHandle::new(path, access_mode, status_flags)?;
        Ok(inode_handle)
    }

    /// Lookup path according to FsPath, always follow symlinks
    pub fn lookup(&self, pathname: &FsPath) -> Result<Arc<Path>> {
        self.lookup_inner(pathname, true)
    }

    /// Lookup path according to FsPath, do not follow it if last component is a symlink
    pub fn lookup_no_follow(&self, pathname: &FsPath) -> Result<Arc<Path>> {
        self.lookup_inner(pathname, false)
    }

    fn lookup_inner(&self, pathname: &FsPath, follow_tail_link: bool) -> Result<Arc<Path>> {
        let path = match pathname.inner {
            FsPathInner::Absolute(pathname) => self.lookup_from_parent(
                &self.root,
                pathname.trim_start_matches('/'),
                follow_tail_link,
            )?,
            FsPathInner::CwdRelative(pathname) => {
                self.lookup_from_parent(&self.cwd, pathname, follow_tail_link)?
            }
            FsPathInner::Cwd => self.cwd.clone(),
            FsPathInner::FdRelative(fd, pathname) => {
                let parent = self.lookup_from_fd(fd)?;
                self.lookup_from_parent(&parent, pathname, follow_tail_link)?
            }
            FsPathInner::Fd(fd) => self.lookup_from_fd(fd)?,
        };

        Ok(path)
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
        parent: &Arc<Path>,
        relative_path: &str,
        follow_tail_link: bool,
    ) -> Result<Arc<Path>> {
        debug_assert!(!relative_path.starts_with('/'));

        if relative_path.len() > PATH_MAX {
            return_errno_with_message!(Errno::ENAMETOOLONG, "path is too long");
        }

        // To handle symlinks
        let mut link_path = String::new();
        let mut follows = 0;

        // Initialize the first path and the relative path
        let (mut path, mut relative_path) = (parent.clone(), relative_path);

        while !relative_path.is_empty() {
            let (next_name, path_remain, must_be_dir) =
                if let Some((prefix, suffix)) = relative_path.split_once('/') {
                    let suffix = suffix.trim_start_matches('/');
                    (prefix, suffix, true)
                } else {
                    (relative_path, "", false)
                };

            // Iterate next dentry
            let next_path = path.lookup(next_name)?;
            let next_dentry = next_path.dentry();
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

                // Change the path and relative path according to symlink
                if link_path_remain.starts_with('/') {
                    path = self.root.clone();
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
                path = next_path;
                relative_path = path_remain;
            }
        }

        Ok(path)
    }

    /// Lookup dentry from the giving fd
    pub fn lookup_from_fd(&self, fd: FileDesc) -> Result<Arc<Dentry>> {
        let current = current!();
        let file_table = current.file_table().lock();
        let inode_handle = file_table
            .get_file(fd)?
            .downcast_ref::<InodeHandle>()
            .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
        Ok(inode_handle.path().clone())
    }

    /// Lookup the dir path and base file name of the giving pathname.
    ///
    /// If the last component is a symlink, do not deference it
    pub fn lookup_dir_and_base_name(&self, pathname: &FsPath) -> Result<(Arc<Path>, String)> {
        self.lookup_dir_and_base_name_inner(pathname, false)
    }

    fn lookup_dir_and_base_name_inner(
        &self,
        pathname: &FsPath,
        follow_tail_link: bool,
    ) -> Result<(Arc<Path>, String)> {
        let (mut dir_path, mut base_name) = match pathname.inner {
            FsPathInner::Absolute(pathname) => {
                let (dir, file_name) = split_path(pathname);
                (
                    self.lookup_from_parent(&self.root, dir.trim_start_matches('/'), true)?,
                    String::from(file_name),
                )
            }
            FsPathInner::CwdRelative(pathname) => {
                let (dir, file_name) = split_path(pathname);
                (
                    self.lookup_from_parent(&self.cwd, dir, true)?,
                    String::from(file_name),
                )
            }
            FsPathInner::FdRelative(fd, pathname) => {
                let (dir, file_name) = split_path(pathname);
                let parent = self.lookup_from_fd(fd)?;
                (
                    self.lookup_from_parent(&parent, dir, true)?,
                    String::from(file_name),
                )
            }
            _ => return_errno!(Errno::ENOENT),
        };
        if !follow_tail_link {
            return Ok((dir_path, base_name));
        }

        // Dereference the tail symlinks if needed
        loop {
            match dir_path.lookup(base_name.trim_end_matches('/')) {
                Ok(path) if path.dentry().type_() == InodeType::SymLink => {
                    let link = {
                        let mut link = path.dentry().inode().read_link()?;
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
                        dir_path =
                            self.lookup_from_parent(&self.root, dir.trim_start_matches('/'), true)?;
                        base_name = String::from(file_name);
                    } else {
                        dir_path = self.lookup_from_parent(&dir_path, dir, true)?;
                        base_name = String::from(file_name);
                    }
                }
                _ => break,
            }
        }

        Ok((dir_path, base_name))
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
