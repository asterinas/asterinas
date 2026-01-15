// SPDX-License-Identifier: MPL-2.0

use alloc::str;

use aster_util::printer::VmPrinter;
use ostd::task::Task;

use super::Path;
use crate::{
    fs::{
        file_table::{FileDesc, get_file_fast},
        path::{MountNamespace, PerMountFlags},
        utils::{FsFlags, InodeType, NAME_MAX, PATH_MAX, Permission, SYMLINKS_MAX, SymbolicLink},
    },
    prelude::*,
    process::posix_thread::AsThreadLocal,
};

/// The file descriptor of the current working directory.
pub const AT_FDCWD: FileDesc = -100;

/// A resolver for [`Path`]s.
///
/// `PathResolver` provides a context for resolving paths, defined by a root directory
/// and a current working directory. It handles path resolution across different mount
/// points and within a specific mount namespace.
///
/// All operations related to path resolution for a process should go through its associated
/// `PathResolver`.
#[derive(Debug, Clone)]
pub struct PathResolver {
    root: Path,
    cwd: Path,
}

impl PathResolver {
    /// Creates a new `PathResolver` with the given `root` and `cwd`.
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

    /// Constructs the absolute path name of a [`Path`].
    ///
    /// This method internally resolves the name and parent of the given `Path`
    /// repeatedly, winding up the path until it reaches the resolver's root or
    /// cannot proceed further.
    ///
    /// # Returns
    ///
    /// - [`AbsPathResult::Reachable`]: The path can be traced back to the resolver's
    ///   root. The returned string is guaranteed to be non-empty and starts with `/`.
    /// - [`AbsPathResult::Unreachable`]: The path cannot be traced back to the resolver's
    ///   root. This can happen for:
    ///   - Pseudo paths that have no parent in the filesystem tree.
    ///   - Paths where the parent chain terminates before reaching the resolver's root.
    ///
    /// For pseudo paths, the returned string is simply the path's name. For other
    /// unreachable paths, the returned string starts with `/` but represents a partial
    /// path that could not reach the root.
    pub fn make_abs_path(&self, path: &Path) -> AbsPathResult {
        // Handle the root path.
        if path == &self.root {
            return AbsPathResult::Reachable("/".to_string());
        }
        // Handle pseudo paths.
        if path.is_pseudo() {
            return AbsPathResult::Unreachable(path.name());
        }

        let mut components = VecDeque::new();
        components.push_front(self.resolve_name(path));

        let mut parent_path = self.resolve_parent(path);
        let mut reach_resolver_root = false;

        while let Some(parent_dir) = parent_path {
            // Stop if we reach the resolver's root.
            if parent_dir == self.root {
                reach_resolver_root = true;
                break;
            }

            let parent_name = self.resolve_name(&parent_dir);
            // Stop if we reach an absolute root.
            if parent_name == "/" {
                break;
            }

            components.push_front(parent_name);

            parent_path = self.resolve_parent(&parent_dir);
        }

        let path_name = alloc::format!("/{}", components.make_contiguous().join("/"));
        debug_assert!(path_name.starts_with('/'));

        if reach_resolver_root {
            AbsPathResult::Reachable(path_name)
        } else {
            AbsPathResult::Unreachable(path_name)
        }
    }

    /// Resolves the name of a `Path`.
    ///
    /// The method resolves the name by the following rules:
    /// 1. If the path is the root of the `PathResolver`, then the name is `"/"`.
    /// 2. If the path is not the root of a mount, then the name is the same as that of the
    ///    underlying dentry.
    /// 3. If the path is the root of a mount and
    ///    - If the mount has a parent mount, then the name is that of the corresponding
    ///      mountpoint in the parent mount.
    ///    - If the mount has no parent, then the name is `"/"`.
    fn resolve_name(&self, path: &Path) -> String {
        let mut owned;
        let mut current = path;

        loop {
            if current == &self.root {
                return "/".to_string();
            }

            if !current.is_mount_root() {
                return current.name();
            }

            let Some(parent) = current.mount_node().parent() else {
                return current.name();
            };
            let Some(mountpoint) = current.mount_node().mountpoint() else {
                return current.name();
            };

            owned = Path::new(parent.upgrade().unwrap(), mountpoint);
            current = &owned;
        }
    }

    /// Resolves the parent of a `Path`.
    ///
    /// The method resolves the parent by the following rules:
    /// 1. If the path is the root of the FS resolver, then the parent is none.
    /// 2. If the path is not the root of a mount, then the parent is the same as that of the
    ///    underlying dentry.
    /// 3. If the path is the root of a mount and
    ///    - If the mount has a parent mount, then the parent is that of the corresponding
    ///      mountpoint in the parent mount.
    ///    - If the mount has no parent, then the parent is none.
    fn resolve_parent(&self, path: &Path) -> Option<Path> {
        // Handle pseudo paths
        if !path.is_mount_root() && path.dentry.parent().is_none() {
            debug_assert!(path.is_pseudo());
            return None;
        }

        let mut owned;
        let mut current = path;

        loop {
            if current == &self.root {
                return None;
            }

            if !current.is_mount_root() {
                return Some(Path::new(
                    current.mount.clone(),
                    current.dentry.parent().unwrap(),
                ));
            }

            let parent = current.mount.parent()?;
            let mountpoint = current.mount.mountpoint()?;

            owned = Path::new(parent.upgrade().unwrap(), mountpoint);
            current = &owned;
        }
    }

    /// Switches the `PathResolver` to the given mount namespace.
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
}

/// The result of resolving an absolute path name.
///
/// If the path can be traced back to the root of the resolver, it is `Reachable`.
/// Otherwise, it is `Unreachable`.
#[derive(Debug, Clone)]
pub enum AbsPathResult {
    Reachable(String),
    Unreachable(String),
}

impl AbsPathResult {
    /// Converts the `AbsPathResult` into a `String`.
    pub fn into_string(self) -> String {
        match self {
            AbsPathResult::Reachable(s) => s,
            AbsPathResult::Unreachable(s) => s,
        }
    }
}

// Mount info reading implementation
impl PathResolver {
    /// Reads the information of the mounts visible to this resolver.
    ///
    /// Here, the visible mounts are defined as follows:
    /// 1. If the resolver's root is a mount point, the visible mounts are the mount of the
    ///    resolver's root directory and all of its descendant mounts in the mount tree.
    /// 2. If the resolver's root is not a mount point, the visible mounts are all descendant
    ///    mounts that are mounted under the resolver's root directory.
    pub fn read_mount_info(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let mut stack = Vec::new();
        if self.root.is_mount_root() {
            stack.push(self.root.mount.clone());
        } else {
            // The root is not a mount root, so we need to find the visible child mounts.
            let children = self.root.mount.children.read();
            for child_mount in children.values() {
                if child_mount
                    .mountpoint()
                    .is_some_and(|dentry| dentry.is_equal_or_descendant_of(&self.root.dentry))
                {
                    stack.push(child_mount.clone());
                }
            }
        }

        while let Some(mount) = stack.pop() {
            let mount_id = mount.id();
            let parent = mount.parent().and_then(|parent| parent.upgrade());
            let parent_id = parent.as_ref().map_or(mount_id, |p| p.id());
            let root = mount.root_dentry().path_name();
            let mount_point = if let Some(parent) = parent {
                if let Some(mount_point_dentry) = mount.mountpoint() {
                    self.make_abs_path(&Path::new(parent, mount_point_dentry))
                        .into_string()
                } else {
                    "".to_string()
                }
            } else {
                // No parent means it's the root of the namespace.
                "/".to_string()
            };
            let mount_flags = mount.flags();
            let fs_type = mount.fs().name();
            let fs_flags = mount.fs().flags();

            // The following fields are dummy for now.
            let major = 0;
            let minor = 0;
            let source = "none";

            let entry = MountInfoEntry {
                mount_id,
                parent_id,
                major,
                minor,
                root: &root,
                mount_point: &mount_point,
                mount_flags,
                fs_type,
                source,
                fs_flags,
            };

            writeln!(printer, "{}", entry)?;

            let children = mount.children.read();
            for child_mount in children.values() {
                stack.push(child_mount.clone());
            }
        }

        Ok(printer.bytes_written())
    }
}

/// A single entry in the mountinfo file.
struct MountInfoEntry<'a> {
    /// A unique ID for the mount (but not guaranteed to be unique across reboots).
    mount_id: usize,
    /// The ID of the parent mount (or self if it has no parent).
    parent_id: usize,
    /// The major device ID of the filesystem.
    major: u32,
    /// The minor device ID of the filesystem.
    minor: u32,
    /// The root of the mount within the filesystem.
    root: &'a str,
    /// The mount point relative to the process's root directory.
    mount_point: &'a str,
    /// Per-mount flags.
    mount_flags: PerMountFlags,
    /// The type of the filesystem in the form "type[.subtype]".
    fs_type: &'a str,
    /// Filesystem-specific information or "none".
    source: &'a str,
    /// Per-filesystem flags.
    fs_flags: FsFlags,
}

impl core::fmt::Display for MountInfoEntry<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} {} {}:{} {} {} {} - {} {} {}",
            self.mount_id,
            self.parent_id,
            self.major,
            self.minor,
            &self.root,
            &self.mount_point,
            &self.mount_flags,
            &self.fs_type,
            &self.source,
            &self.fs_flags,
        )
    }
}

// Path lookup implementations
impl PathResolver {
    /// Looks up a child entry with `name` within a directory `path`.
    pub fn lookup_at_path(&self, path: &Path, name: &str) -> Result<Path> {
        if path.type_() != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "the path is not a directory");
        }
        if path.inode().check_permission(Permission::MAY_EXEC).is_err() {
            return_errno_with_message!(Errno::EACCES, "the path cannot be looked up");
        }
        if name.len() > NAME_MAX {
            return_errno_with_message!(Errno::ENAMETOOLONG, "the path name is too long");
        }

        let target_path = if super::is_dot(name) {
            path.this()
        } else if super::is_dotdot(name) {
            self.resolve_parent(path).unwrap_or_else(|| path.this())
        } else {
            let target_inner_opt = path.dentry.lookup_via_cache(name)?;
            match target_inner_opt {
                Some(target_inner) => Path::new(path.mount.clone(), target_inner),
                None => {
                    let target_inner = path.dentry.lookup_via_fs(name)?;
                    Path::new(path.mount.clone(), target_inner)
                }
            }
        };

        Ok(target_path.get_top_path())
    }

    /// Lookups the target `Path` according to the `fs_path`.
    ///
    /// Symlinks are always followed.
    pub fn lookup(&self, fs_path: &FsPath) -> Result<Path> {
        self.lookup_unresolved(fs_path)?.into_path()
    }

    /// Lookups the target `Path` according to the `fs_path`.
    ///
    /// If the last component is a symlink, it will not be followed.
    pub fn lookup_no_follow(&self, fs_path: &FsPath) -> Result<Path> {
        self.lookup_unresolved_no_follow(fs_path)?.into_path()
    }

    /// Lookups the target `Path` according to the `fs_path` and leaves
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

    /// Lookups the target `Path` according to the `fs_path` and leaves
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
        let lookup_res = match fs_path.inner {
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
                LookupResult::Resolved(file.path().clone())
            }
        };

        Ok(lookup_res)
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
            let next_path = match self.lookup_at_path(&current_path, next_name) {
                Ok(child) => child,
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
                let read_link_res = next_path.inode().read_link()?;
                match read_link_res {
                    SymbolicLink::Plain(mut tmp_link_path) => {
                        let link_path_remain = {
                            if tmp_link_path.is_empty() {
                                return_errno_with_message!(
                                    Errno::ENOENT,
                                    "the symlink path is empty"
                                );
                            }
                            if !next_is_tail {
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
                    }
                    SymbolicLink::Path(path) => {
                        current_path = path;
                        relative_path = path_remain;
                        follows += 1;
                    }
                }
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
            LookupResult::Resolved(target) => Ok(target),
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

/// A path in the file system.
#[derive(Debug)]
pub struct FsPath<'a> {
    inner: FsPathInner<'a>,
}

#[derive(Debug)]
enum FsPathInner<'a> {
    /// An absolute path.
    Absolute(&'a str),
    /// A relative path from the current working directory.
    CwdRelative(&'a str),
    /// The path of the current working directory.
    Cwd,
    /// A relative path from the directory FD (dirfd).
    FdRelative(FileDesc, &'a str),
    /// The path of the FD.
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
