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
                let parent = file.as_inode_handle_or_err()?.path();
                self.lookup_from_parent(parent, path, follow_tail_link)?
            }
            FsPathInner::Fd(fd) => {
                let task = Task::current().unwrap();
                let mut file_table = task.as_thread_local().unwrap().borrow_file_table_mut();
                let file = get_file_fast!(&mut file_table, fd);
                LookupResult::Resolved(file.as_inode_handle_or_err()?.path().clone())
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
            return_errno_with_message!(Errno::ENAMETOOLONG, "the path is too long");
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
                    return_errno_with_message!(Errno::ELOOP, "there are too many symlinks");
                }
                let link_path_remain = {
                    let mut tmp_link_path = next_path.inode().read_link()?;
                    if tmp_link_path.is_empty() {
                        return_errno_with_message!(Errno::ENOENT, "the symlink path is empty");
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
                    return_errno_with_message!(Errno::ENOTDIR, "the inode is not a directory");
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
    /// Creates a new `FsPath` from the given `dirfd`.
    ///
    /// If the FD is not valid (i.e., it's negative and it's not [`AT_FDCWD`]), an error will be
    /// returned.
    pub fn from_fd(dirfd: FileDesc) -> Result<Self> {
        let fs_path_inner = if dirfd >= 0 {
            FsPathInner::Fd(dirfd)
        } else if dirfd == AT_FDCWD {
            FsPathInner::Cwd
        } else {
            return_errno_with_message!(Errno::EBADF, "the dirfd is invalid");
        };

        Ok(Self {
            inner: fs_path_inner,
        })
    }

    /// Creates a new `FsPath` from the given `dirfd` and `path`.
    ///
    /// If the FD is not valid (i.e., it's negative and it's not [`AT_FDCWD`]) or the path is empty
    /// or too long, an error will be returned.
    pub fn from_fd_and_path(dirfd: FileDesc, path: &'a str) -> Result<Self> {
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "the path is empty")
        }
        if path.len() > PATH_MAX {
            return_errno_with_message!(Errno::ENAMETOOLONG, "the path is too long");
        }

        let fs_path_inner = if path.starts_with('/') {
            FsPathInner::Absolute(path)
        } else if dirfd >= 0 {
            FsPathInner::FdRelative(dirfd, path)
        } else if dirfd == AT_FDCWD {
            FsPathInner::CwdRelative(path)
        } else {
            return_errno_with_message!(Errno::EBADF, "the dirfd is invalid");
        };

        Ok(Self {
            inner: fs_path_inner,
        })
    }
}

impl<'a> TryFrom<&'a str> for FsPath<'a> {
    type Error = crate::error::Error;

    fn try_from(path: &'a str) -> Result<FsPath<'a>> {
        FsPath::from_fd_and_path(AT_FDCWD, path)
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

    /// Consumes the `LookupResult` and returns the parent path and the unresolved file name.
    ///
    /// If the path was resolved or the unresolved name was expected to be a directory, an error
    /// will be returned.
    pub fn into_parent_and_filename(self) -> Result<(Path, String)> {
        let LookupResult::AtParent(res) = self else {
            return_errno_with_message!(Errno::EEXIST, "the path already exists");
        };
        res.into_parent_and_filename()
    }

    /// Consumes the `LookupResult` and returns the parent path and the unresolved name.
    ///
    /// If the path was resolved, an error will be returned.
    pub fn into_parent_and_basename(self) -> Result<(Path, String)> {
        let LookupResult::AtParent(res) = self else {
            return_errno_with_message!(Errno::EEXIST, "the path already exists");
        };
        Ok(res.into_parent_and_basename())
    }
}

/// A result that contains information about a path lookup that stopped
/// at a parent directory.
pub struct LookupParentResult {
    /// The path of the parent directory where resolution stopped.
    parent: Path,
    /// The remaining unresolved component name.
    unresolved_name: String,
    /// Indicates whether the target was expected to be a directory.
    target_is_dir: bool,
}

impl LookupParentResult {
    fn new(parent: Path, unresolved_name: String, target_is_dir: bool) -> Self {
        Self {
            parent,
            unresolved_name,
            target_is_dir,
        }
    }

    /// Returns true if the target was expected to be a directory.
    pub fn target_is_dir(&self) -> bool {
        self.target_is_dir
    }

    /// Consumes the `LookupParentResult` and returns the parent path and the unresolved file name.
    ///
    /// If the unresolved name was expected to be a directory, an error will be returned.
    pub fn into_parent_and_filename(self) -> Result<(Path, String)> {
        if self.target_is_dir {
            return_errno_with_message!(Errno::ENOENT, "the path is a directory");
        }
        Ok((self.parent, self.unresolved_name))
    }

    /// Consumes the `LookupParentResult` and returns the parent path and the unresolved name.
    pub fn into_parent_and_basename(self) -> (Path, String) {
        (self.parent, self.unresolved_name)
    }
}

/// Utilities to split a string into its path components.
pub trait SplitPath {
    /// Splits a path into the parent directory name and the final component name, which is
    /// expected to be a file (not a directory).
    ///
    /// If the final component refers to a directory, an error will be returned. Aside from the
    /// constraint on the final component, this is similar to [`Self::split_dirname_and_basename`].
    fn split_dirname_and_filename(&self) -> Result<(&Self, &Self)>;

    /// Splits a path into the parent directory name and the final component name.
    ///
    /// This behaves in a similar way to the POSIX C functions [`dirname()` and
    /// `basename()`](https://man7.org/linux/man-pages/man3/basename.3.html). Trailing slashes
    /// (`/`) are trimmed from returned names unless the name refers to the root directory. In that
    /// case, the name contains a single slash.
    ///
    /// If the original path is an empty string, an error will be returned.
    ///
    /// If the original path directly points to the root directory (e.g., `/` or `//`, but not `/.`
    /// or `/./`), an error will be returned.
    fn split_dirname_and_basename(&self) -> Result<(&Self, &Self)>;
}

impl SplitPath for str {
    fn split_dirname_and_filename(&self) -> Result<(&Self, &Self)> {
        if self.ends_with('/') {
            return_errno_with_message!(Errno::EISDIR, "the path is a directory");
        }

        self.split_dirname_and_basename()
    }

    fn split_dirname_and_basename(&self) -> Result<(&Self, &Self)> {
        if self.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "the path is empty");
        }

        let trimmed = self.trim_end_matches('/');
        if trimmed.is_empty() {
            return_errno_with_message!(Errno::EBUSY, "the path is the root directory");
        }

        if let Some(pos) = trimmed.rfind('/') {
            let dirname = trimmed[..pos].trim_end_matches('/');
            Ok((
                if dirname.is_empty() { "/" } else { dirname },
                &trimmed[pos + 1..],
            ))
        } else {
            Ok((".", trimmed))
        }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;

    type SplitResult = core::result::Result<(&'static str, &'static str), Errno>;

    #[track_caller]
    fn assert_split_results(
        cases: &Vec<(&'static str, SplitResult)>,
        split: impl Fn(&str) -> Result<(&str, &str)>,
    ) {
        for case in cases.iter() {
            let result = split(case.0);
            assert_eq!(
                result.map_err(|err| err.error()),
                case.1,
                "splitting '{}' failed",
                case.0
            );
        }
    }

    #[ktest]
    fn path_split_filename() {
        let cases = vec![
            ("", Err(Errno::ENOENT)),
            ("/", Err(Errno::EISDIR)),
            ("///", Err(Errno::EISDIR)),
            ("///.", Ok(("/", "."))),
            ("//./", Err(Errno::EISDIR)),
            ("a", Ok((".", "a"))),
            ("/a", Ok(("/", "a"))),
            ("b/a", Ok(("b", "a"))),
            ("/b/a", Ok(("/b", "a"))),
            ("a", Ok((".", "a"))),
            ("//a", Ok(("/", "a"))),
            ("b//a", Ok(("b", "a"))),
            ("//b//a", Ok(("//b", "a"))),
            ("a/", Err(Errno::EISDIR)),
            ("/a/", Err(Errno::EISDIR)),
            ("b/a/", Err(Errno::EISDIR)),
            ("/b/a/", Err(Errno::EISDIR)),
            ("a//", Err(Errno::EISDIR)),
            ("/a//", Err(Errno::EISDIR)),
            ("b/a//", Err(Errno::EISDIR)),
            ("/b/a//", Err(Errno::EISDIR)),
            (" a ", Ok((".", " a "))),
            (" //a ", Ok((" ", "a "))),
            (" b//a ", Ok((" b", "a "))),
            (" //b//a ", Ok((" //b", "a "))),
            (" a/ ", Ok((" a", " "))),
            (" //a/ ", Ok((" //a", " "))),
            (" b//a/ ", Ok((" b//a", " "))),
            (" //b//a/ ", Ok((" //b//a", " "))),
        ];
        assert_split_results(&cases, SplitPath::split_dirname_and_filename);
    }

    #[ktest]
    fn path_split_basename() {
        let cases = vec![
            ("", Err(Errno::ENOENT)),
            ("/", Err(Errno::EBUSY)),
            ("///", Err(Errno::EBUSY)),
            ("///.", Ok(("/", "."))),
            ("//./", Ok(("/", "."))),
            ("a", Ok((".", "a"))),
            ("/a", Ok(("/", "a"))),
            ("b/a", Ok(("b", "a"))),
            ("/b/a", Ok(("/b", "a"))),
            ("a", Ok((".", "a"))),
            ("//a", Ok(("/", "a"))),
            ("b//a", Ok(("b", "a"))),
            ("//b//a", Ok(("//b", "a"))),
            ("a/", Ok((".", "a"))),
            ("/a/", Ok(("/", "a"))),
            ("b/a/", Ok(("b", "a"))),
            ("/b/a/", Ok(("/b", "a"))),
            ("a//", Ok((".", "a"))),
            ("/a//", Ok(("/", "a"))),
            ("b/a//", Ok(("b", "a"))),
            ("/b/a//", Ok(("/b", "a"))),
            (" a ", Ok((".", " a "))),
            (" //a ", Ok((" ", "a "))),
            (" b//a ", Ok((" b", "a "))),
            (" //b//a ", Ok((" //b", "a "))),
            (" a/ ", Ok((" a", " "))),
            (" //a/ ", Ok((" //a", " "))),
            (" b//a/ ", Ok((" b//a", " "))),
            (" //b//a/ ", Ok((" //b//a", " "))),
        ];
        assert_split_results(&cases, SplitPath::split_dirname_and_basename);
    }
}
