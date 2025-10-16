// SPDX-License-Identifier: MPL-2.0

use alloc::str;

use ostd::task::Task;

use super::{
    file_table::{get_file_fast, FileDesc},
    path::Path,
    utils::{InodeType, PATH_MAX, SYMLINKS_MAX},
};
use crate::{fs::path::MountNamespace, prelude::*, process::posix_thread::AsThreadLocal};

/// The file descriptor of the current working directory.
pub const AT_FDCWD: FileDesc = -100;

/// File system resolver.
#[derive(Debug, Clone)]
pub struct FsResolver {
    root: Path,
    cwd: Path,
}

impl FsResolver {
    /// Creates a new `FsResolver` with the given `root` and `cwd`.
    pub(super) fn new(root: Path, cwd: Path) -> Self {
        Self { root, cwd }
    }

    /// Gets the path of the root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Gets the path of the current working directory.
    #[expect(dead_code)]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Sets the current working directory to the given `path`.
    pub fn set_cwd(&mut self, path: Path) {
        self.cwd = path;
    }

    /// Sets the root directory to the given `path`.
    pub fn set_root(&mut self, path: Path) {
        self.root = path;
    }

    /// Switches the `FsResolver` to the given mount namespace.
    ///
    /// If the target namespace already owns both the current root and working directory's
    /// mount nodes, the operation is a no-op and returns immediately.
    ///
    /// Otherwise, the method will change both the `cwd` and `root` to their corresponding
    /// `Path`s within the new `MountNamespace`.
    //
    // FIXME: This cannot fail if we clone mount namespaces and update resolvers in an atomic way.
    // We currently leak this error to userspace, which is not a correct behavior.
    pub fn switch_to_mnt_ns(&mut self, mnt_ns: &Arc<MountNamespace>) -> Result<()> {
        if mnt_ns.owns(self.root.mount_node()) && mnt_ns.owns(self.cwd.mount_node()) {
            return Ok(());
        }

        let new_root = self.root.find_corresponding_mount(mnt_ns).ok_or_else(|| {
            Error::with_message(
                Errno::EINVAL,
                "the root directory does not exist in the target mount namespace",
            )
        })?;
        let new_cwd = self.cwd.find_corresponding_mount(mnt_ns).ok_or_else(|| {
            Error::with_message(
                Errno::EINVAL,
                "the current working directory does not exist in the target mount namespace",
            )
        })?;

        self.root = new_root;
        self.cwd = new_cwd;

        Ok(())
    }

    /// Lookups the target path according to the `fs_path`.
    ///
    /// Symlinks are always followed.
    pub fn lookup(&self, fs_path: &FsPath) -> Result<Path> {
        self.lookup_inner(fs_path, true)?.into_path()
    }

    /// Lookups the target path according to the `fs_path`.
    ///
    /// If the last component is a symlink, it will not be followed.
    pub fn lookup_no_follow(&self, fs_path: &FsPath) -> Result<Path> {
        self.lookup_inner(fs_path, false)?.into_path()
    }

    /// Lookups the target path according to the `fs_path` and leaves
    /// the result unresolved.
    ///
    /// An unresolved result may indicate either successful full-path resolution,
    /// or resolution stopping at a parent path when the target doesn't exist
    /// but its parent does.
    ///
    /// Symlinks are always followed.
    pub fn lookup_unresolved(&self, fs_path: &FsPath) -> Result<LookupResult> {
        self.lookup_inner(fs_path, true)
    }

    /// Lookups the target path according to the `fs_path` and leaves
    /// the result unresolved.
    ///
    /// An unresolved result may indicate either successful full-path resolution,
    /// or resolution stopping at a parent path when the target doesn't exist
    /// but its parent does.
    ///
    /// If the last component is a symlink, it will not be followed.
    pub fn lookup_unresolved_no_follow(&self, fs_path: &FsPath) -> Result<LookupResult> {
        self.lookup_inner(fs_path, false)
    }

    fn lookup_inner(&self, fs_path: &FsPath, follow_tail_link: bool) -> Result<LookupResult> {
        let path = match fs_path.inner {
            FsPathInner::Absolute(path) => {
                self.lookup_from_parent(&self.root, path.trim_start_matches('/'), follow_tail_link)?
            }
            FsPathInner::CwdRelative(path) => {
                self.lookup_from_parent(&self.cwd, path, follow_tail_link)?
            }
            FsPathInner::Cwd => LookupResult::Resolved(self.cwd.clone()),
            FsPathInner::FdRelative(fd, path) => {
                let task = Task::current().unwrap();
                let mut file_table = task.as_thread_local().unwrap().borrow_file_table_mut();
                let file = get_file_fast!(&mut file_table, fd);
                self.lookup_from_parent(file.as_inode_or_err()?.path(), path, follow_tail_link)?
            }
            FsPathInner::Fd(fd) => {
                let task = Task::current().unwrap();
                let mut file_table = task.as_thread_local().unwrap().borrow_file_table_mut();
                let file = get_file_fast!(&mut file_table, fd);
                LookupResult::Resolved(file.as_inode_or_err()?.path().clone())
            }
        };

        Ok(path)
    }

    /// Lookups the target path according to the parent directory path.
    ///
    /// The length of `path` cannot exceed `PATH_MAX`.
    /// If `path` ends with `/`, then the returned inode must be a directory inode.
    ///
    /// While looking up the path, symbolic links will be followed for
    /// at most `SYMLINKS_MAX` times.
    ///
    /// If `follow_tail_link` is true and the trailing component is a symlink,
    /// it will be followed.
    /// Symlinks in earlier components of the path will always be followed.
    #[expect(clippy::redundant_closure)]
    fn lookup_from_parent(
        &self,
        parent: &Path,
        relative_path: &str,
        follow_tail_link: bool,
    ) -> Result<LookupResult> {
        debug_assert!(!relative_path.starts_with('/'));

        if relative_path.len() > PATH_MAX {
            return_errno_with_message!(Errno::ENAMETOOLONG, "path is too long");
        }
        if relative_path.is_empty() {
            return Ok(LookupResult::Resolved(parent.clone()));
        }

        // To handle symlinks
        let mut link_path_opt = None;
        let mut follows = 0;

        // Initialize the first path and the relative path
        let (mut current_path, mut relative_path) = (parent.clone(), relative_path);

        while !relative_path.is_empty() {
            let (next_name, path_remain, target_is_dir) =
                if let Some((prefix, suffix)) = relative_path.split_once('/') {
                    let suffix = suffix.trim_start_matches('/');
                    (prefix, suffix, true)
                } else {
                    (relative_path, "", false)
                };

            // Iterate next path
            let next_is_tail = path_remain.is_empty();
            let next_path = match current_path.lookup(next_name) {
                Ok(current_dir) => current_dir,
                Err(e) => {
                    if next_is_tail && e.error() == Errno::ENOENT {
                        return Ok(LookupResult::AtParent(LookupParentResult::new(
                            current_path,
                            next_name.to_string(),
                            target_is_dir,
                        )));
                    }
                    return Err(e);
                }
            };

            let next_type = next_path.type_();
            // If next inode is a symlink, follow symlinks at most `SYMLINKS_MAX` times.
            if next_type == InodeType::SymLink && (follow_tail_link || !next_is_tail) {
                if follows >= SYMLINKS_MAX {
                    return_errno_with_message!(Errno::ELOOP, "too many symlinks");
                }
                let link_path_remain = {
                    let mut tmp_link_path = next_path.inode().read_link()?;
                    if tmp_link_path.is_empty() {
                        return_errno_with_message!(Errno::ENOENT, "empty symlink");
                    }
                    if !path_remain.is_empty() {
                        tmp_link_path += "/";
                        tmp_link_path += path_remain;
                    } else if target_is_dir {
                        tmp_link_path += "/";
                    }
                    tmp_link_path
                };

                // Change the path and relative path according to symlink
                if link_path_remain.starts_with('/') {
                    current_path = self.root.clone();
                }
                let link_path = link_path_opt.get_or_insert_with(|| String::new());
                link_path.clear();
                link_path.push_str(link_path_remain.trim_start_matches('/'));
                relative_path = link_path;
                follows += 1;
            } else {
                // If path ends with `/`, the inode must be a directory
                if target_is_dir && next_type != InodeType::Dir {
                    return_errno_with_message!(Errno::ENOTDIR, "inode is not dir");
                }
                current_path = next_path;
                relative_path = path_remain;
            }
        }

        Ok(LookupResult::Resolved(current_path))
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

// A result type for lookup operations.
pub enum LookupResult {
    /// The entire path was resolved to a final `Path`.
    Resolved(Path),
    /// The path resolution stopped at a parent directory.
    AtParent(LookupParentResult),
}

impl LookupResult {
    fn into_path(self) -> Result<Path> {
        match self {
            LookupResult::Resolved(path) => Ok(path),
            LookupResult::AtParent(_) => Err(Error::with_message(
                Errno::ENOENT,
                "path resolution did not reach the final target",
            )),
        }
    }

    /// Consumes the `LookupResult` and returns the parent path and the tail file name.
    ///
    /// If the path was resolved or the target was expected to be a directory,
    /// an error will be returned.
    pub fn into_parent_and_tail_filename(self) -> Result<(Path, String)> {
        let LookupResult::AtParent(res) = self else {
            return_errno_with_message!(Errno::EEXIST, "the path already exists");
        };
        res.into_parent_and_tail_filename()
    }

    /// Consumes the `LookupResult` and returns the parent path and the tail name.
    ///
    /// If the path was resolved, an error will be returned.
    pub fn into_parent_and_tail_name(self) -> Result<(Path, String)> {
        let LookupResult::AtParent(res) = self else {
            return_errno_with_message!(Errno::EEXIST, "the path already exists");
        };
        Ok(res.into_parent_and_tail_name())
    }
}

/// A result that contains information about a path lookup that stopped
/// at a parent directory.
pub struct LookupParentResult {
    /// The path of the parent directory where resolution stopped.
    parent: Path,
    /// The remaining unresolved component name.
    tail_name: String,
    /// Indicates whether the target was expected to be a directory.
    target_is_dir: bool,
}

impl LookupParentResult {
    fn new(parent: Path, tail_name: String, target_is_dir: bool) -> Self {
        Self {
            parent,
            tail_name,
            target_is_dir,
        }
    }

    /// Returns true if the target was expected to be a directory.
    pub fn target_is_dir(&self) -> bool {
        self.target_is_dir
    }

    /// Consumes the `LookupParentResult` and returns the parent path and the tail file name.
    ///
    /// If the target was expected to be a directory, an error will be returned.
    pub fn into_parent_and_tail_filename(self) -> Result<(Path, String)> {
        if self.target_is_dir {
            return_errno_with_message!(Errno::ENOENT, "the path is a directory");
        }
        Ok((self.parent, self.tail_name))
    }

    /// Consumes the `LookupParentResult` and returns the parent path and the tail name.
    pub fn into_parent_and_tail_name(self) -> (Path, String) {
        (self.parent, self.tail_name)
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
        .next_back()
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
