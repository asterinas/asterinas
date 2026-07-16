// SPDX-License-Identifier: MPL-2.0

//! The device temporary filesystem.
//!
//! This module implements `devtmpfs` as a singleton filesystem backed by
//! `tmpfs`. Device subsystems submit node-management requests through
//! [`create_node`] and [`delete_node`]; the requests are serialized and handled
//! by the dedicated kernel thread `devtmpfsd`.
//!
//! Mounting policy follows the selected init path. If an initramfs init is
//! selected, either by `rdinit=` or by the default `/init` lookup, the kernel
//! does not mount devtmpfs automatically; the selected initramfs init program is
//! responsible for mounting it if needed. Otherwise, Asterinas boots from the
//! configured root filesystem and the kernel mounts this singleton on `/dev`
//! during first-process initialization.

use alloc::borrow::Cow;

use device_id::DeviceId;
use ostd::sync::WaitQueue;
use spin::Once;

use crate::{
    device::DeviceType,
    fs::{
        file::{InodeMode, InodeType, mkmod},
        fs_impls::ramfs::RamInode,
        tmpfs::TmpFs,
        vfs::{
            file_system::FileSystem,
            inode::{Inode, MknodType, RevalidationPolicy},
            path,
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
    thread::kernel_thread::ThreadOptions,
};

/// The metadata that describes a device inode in devtmpfs.
///
/// The metadata contains the inode path relative to `/dev` and the permission
/// bits used when creating the inode. Device subsystems can use this type to
/// override the default mode.
///
/// If a device does not specify a mode explicitly, we use `mkmod!(u+rw)`,
/// matching Linux devtmpfs's default device inode permissions.
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/base/devtmpfs.c#L11>.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevtmpfsInodeMeta<'a> {
    path: Cow<'a, str>,
    mode: InodeMode,
}

impl<'a> DevtmpfsInodeMeta<'a> {
    /// Creates the metadata for a devtmpfs inode with the default mode (`u+rw`).
    pub fn new(path: impl Into<Cow<'a, str>>) -> Self {
        Self {
            path: path.into(),
            mode: mkmod!(u+rw),
        }
    }

    /// Creates the metadata for a devtmpfs inode with the specified path and mode.
    pub fn with_mode(path: impl Into<Cow<'a, str>>, mode: InodeMode) -> Self {
        Self {
            path: path.into(),
            mode,
        }
    }

    /// Returns the device inode path relative to `/dev`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the permission bits of the device inode.
    pub fn mode(&self) -> InodeMode {
        self.mode
    }
}

/// The complete description of a device node managed by devtmpfs.
pub struct DevtmpfsNode {
    device_type: DeviceType,
    device_id: DeviceId,
    meta: DevtmpfsInodeMeta<'static>,
}

impl DevtmpfsNode {
    pub fn new(
        device_type: DeviceType,
        device_id: DeviceId,
        meta: DevtmpfsInodeMeta<'static>,
    ) -> Self {
        Self {
            device_type,
            device_id,
            meta,
        }
    }
}

pub(in crate::fs) fn singleton() -> &'static Arc<TmpFs> {
    static SINGLETON: Once<Arc<TmpFs>> = Once::new();

    SINGLETON.call_once(|| {
        // devtmpfsd creates and deletes device nodes from a kernel thread,
        // outside the VFS path operation that may have cached the dentry.
        // Revalidate directory entries so cached positive/negative dentries
        // reflect the latest devtmpfs tree.
        TmpFs::new_tmpfs_backing(
            "devtmpfs",
            RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT,
        )
    })
}

/// Creates a device node through `devtmpfsd`.
///
/// The request is queued to the dedicated devtmpfs kernel thread and this
/// function waits until the node has been created or the creation fails.
pub fn create_node(node: DevtmpfsNode) -> Result<()> {
    submit(Request::CreateNode(node))
}

/// Deletes a device node through `devtmpfsd`.
///
/// The request is queued to the dedicated devtmpfs kernel thread and this
/// function waits until the deletion has completed or failed. The deletion only
/// unlinks nodes that were created by `devtmpfsd` and still match the requested
/// device type and device ID.
pub fn delete_node(node: DevtmpfsNode) -> Result<()> {
    submit(Request::DeleteNode(node))
}

pub(super) fn init() {
    crate::fs::vfs::registry::register(&DevTmpFsType).unwrap();
}

pub(super) fn init_in_first_kthread() {
    ThreadOptions::new(devtmpfsd).spawn();
}

fn submit(request: Request) -> Result<()> {
    let request = Arc::new(PendingRequest::new(request));
    let queue = request_queue();
    queue.requests.lock().push_back(request.clone());
    queue.wait_queue.wake_one();

    request
        .wait_queue
        .wait_until(|| request.result.lock().take())
}

fn devtmpfsd() {
    let queue = request_queue();

    loop {
        let request = queue
            .wait_queue
            .wait_until(|| queue.requests.lock().pop_front());
        let result = handle_request(&request.request);

        *request.result.lock() = Some(result);
        request.wait_queue.wake_all();
    }
}

fn request_queue() -> &'static RequestQueue {
    static REQUEST_QUEUE: Once<RequestQueue> = Once::new();

    REQUEST_QUEUE.call_once(|| RequestQueue {
        requests: SpinLock::new(VecDeque::new()),
        wait_queue: WaitQueue::new(),
    })
}

struct RequestQueue {
    requests: SpinLock<VecDeque<Arc<PendingRequest>>>,
    wait_queue: WaitQueue,
}

struct PendingRequest {
    request: Request,
    result: Mutex<Option<Result<()>>>,
    wait_queue: WaitQueue,
}

impl PendingRequest {
    fn new(request: Request) -> Self {
        Self {
            request,
            result: Mutex::new(None),
            wait_queue: WaitQueue::new(),
        }
    }
}

enum Request {
    CreateNode(DevtmpfsNode),
    DeleteNode(DevtmpfsNode),
}

fn root_inode() -> Arc<dyn Inode> {
    singleton().root_inode()
}

fn handle_request(request: &Request) -> Result<()> {
    match request {
        Request::CreateNode(node) => add_node(node),
        Request::DeleteNode(node) => remove_node(node),
    }
}

fn add_node(node: &DevtmpfsNode) -> Result<()> {
    let mut parent_inode = root_inode();
    let mut relative_path = normalize_node_path(node.meta.path())?;

    while let Some((next_name, path_remain)) = next_path_component(relative_path) {
        reject_special_component(next_name)?;
        match parent_inode.lookup(next_name) {
            Ok(next_inode) => {
                if path_remain.is_empty() {
                    return_errno_with_message!(Errno::EEXIST, "the device node already exists");
                }
                parent_inode = next_inode;
            }
            Err(error) if error.error() == Errno::ENOENT => {
                if path_remain.is_empty() {
                    let rdev = node.device_id.as_encoded_u64();
                    let mknod_type = match &node.device_type {
                        DeviceType::Block => MknodType::BlockDevice(rdev),
                        DeviceType::Char => MknodType::CharDevice(rdev),
                    };
                    match parent_inode.mknod(next_name, node.meta.mode(), mknod_type) {
                        Ok(new_inode) => mark_kernel_managed(new_inode.as_ref()),
                        Err(error) if error.error() == Errno::EEXIST => {}
                        Err(error) => return Err(error),
                    }
                } else {
                    match parent_inode.create(next_name, InodeType::Dir, mkmod!(a+rx, u+w)) {
                        Ok(new_inode) => {
                            mark_kernel_managed(new_inode.as_ref());
                            parent_inode = new_inode;
                        }
                        Err(error) if error.error() == Errno::EEXIST => {
                            let existing_inode = parent_inode.lookup(next_name)?;
                            if existing_inode.type_() != InodeType::Dir {
                                return_errno_with_message!(
                                    Errno::ENOTDIR,
                                    "the parent path is not a directory"
                                );
                            }
                            parent_inode = existing_inode;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            Err(error) => return Err(error),
        }

        relative_path = path_remain;
    }

    Ok(())
}

fn remove_node(node: &DevtmpfsNode) -> Result<()> {
    let relative_path = normalize_node_path(node.meta.path())?;
    let Some((parent, node_name)) = split_parent_and_basename(relative_path) else {
        return_errno_with_message!(Errno::EINVAL, "the device path is invalid");
    };

    let parent_inode = if parent.is_empty() {
        root_inode()
    } else {
        lookup_relative(parent)?
    };
    reject_special_component(node_name)?;
    let parent_ram_inode = devtmpfs_backing_inode(parent_inode.as_ref());

    if parent_ram_inode.unlink_if(node_name, |inode| Ok(matches_devtmpfs_node(inode, node)))? {
        remove_empty_parent_dirs(parent);
    }

    Ok(())
}

fn remove_empty_parent_dirs(path: &str) {
    let mut path = path;

    while let Some((parent_path, name)) = split_parent_and_basename(path) {
        let parent_inode = if parent_path.is_empty() {
            root_inode()
        } else {
            match lookup_relative(parent_path) {
                Ok(inode) => inode,
                Err(_) => break,
            }
        };

        let parent_ram_inode = devtmpfs_backing_inode(parent_inode.as_ref());
        match parent_ram_inode.rmdir_if(name, |inode| Ok(inode.is_kernel_managed())) {
            Ok(true) => {}
            _ => break,
        }

        path = parent_path;
    }
}

fn matches_devtmpfs_node(inode: &RamInode, node: &DevtmpfsNode) -> bool {
    if !inode.is_kernel_managed() {
        return false;
    }

    let expected_type = match node.device_type {
        DeviceType::Block => InodeType::BlockDevice,
        DeviceType::Char => InodeType::CharDevice,
    };

    inode.type_() == expected_type && inode.metadata().self_dev_id == Some(node.device_id)
}

fn lookup_relative(path: &str) -> Result<Arc<dyn Inode>> {
    let mut current = root_inode();
    let mut relative_path = path;

    while let Some((next_name, path_remain)) = next_path_component(relative_path) {
        reject_special_component(next_name)?;
        current = current.lookup(next_name)?;
        relative_path = path_remain;
    }

    Ok(current)
}

fn reject_special_component(name: &str) -> Result<()> {
    if path::is_dot_or_dotdot(name) {
        return_errno_with_message!(Errno::EINVAL, "special path components are not allowed");
    }

    Ok(())
}

fn normalize_node_path(path: &str) -> Result<&str> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the device path is invalid");
    }

    Ok(path)
}

fn next_path_component(path: &str) -> Option<(&str, &str)> {
    if path.is_empty() {
        return None;
    }

    Some(if let Some((prefix, suffix)) = path.split_once('/') {
        (prefix, suffix.trim_start_matches('/'))
    } else {
        (path, "")
    })
}

fn split_parent_and_basename(path: &str) -> Option<(&str, &str)> {
    if path.is_empty() {
        return None;
    }

    path.rsplit_once('/').map_or_else(
        || Some(("", path)),
        |(parent, basename)| (!basename.is_empty()).then_some((parent, basename)),
    )
}

fn devtmpfs_backing_inode(inode: &dyn Inode) -> &RamInode {
    // devtmpfs is backed by tmpfs, which currently aliases ramfs. Therefore
    // all devtmpfs backing inodes are RamInodes.
    inode
        .downcast_ref::<RamInode>()
        .expect("devtmpfs backing inode must be a RamInode")
}

fn mark_kernel_managed(inode: &dyn Inode) {
    devtmpfs_backing_inode(inode).mark_kernel_managed();
}

struct DevTmpFsType;

impl FsType for DevTmpFsType {
    fn name(&self) -> &'static str {
        "devtmpfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, _fs_creation_ctx: &FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        Ok(singleton().clone())
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

#[cfg(ktest)]
mod tests {
    use device_id::{MajorId, MinorId};
    use ostd::prelude::ktest;

    use super::*;

    fn init() {
        static START: Once<()> = Once::new();

        crate::time::clocks::init_for_ktest();
        START.call_once(|| {
            ThreadOptions::new(devtmpfsd).spawn();
        });
    }

    fn device_id(major: u16, minor: u32) -> DeviceId {
        DeviceId::new(MajorId::new(major), MinorId::new(minor))
    }

    fn char_node(path: &'static str, major: u16, minor: u32) -> DevtmpfsNode {
        DevtmpfsNode::new(
            DeviceType::Char,
            device_id(major, minor),
            DevtmpfsInodeMeta::with_mode(path, mkmod!(a+rw)),
        )
    }

    fn assert_missing(path: &str) {
        match lookup_relative(path) {
            Err(error) => assert_eq!(error.error(), Errno::ENOENT),
            Ok(_) => panic!("path {path} should not exist"),
        }
    }

    fn is_kernel_managed(inode: &dyn Inode) -> bool {
        devtmpfs_backing_inode(inode).is_kernel_managed()
    }

    #[ktest]
    fn create_node_creates_marked_dirs_and_device_node() {
        init();

        let path = "__devtmpfs_ktest_create/input/event0";
        create_node(char_node(path, 240, 1)).unwrap();

        let top_dir = lookup_relative("__devtmpfs_ktest_create").unwrap();
        let input_dir = lookup_relative("__devtmpfs_ktest_create/input").unwrap();
        let event_node = lookup_relative(path).unwrap();

        assert_eq!(top_dir.type_(), InodeType::Dir);
        assert_eq!(input_dir.type_(), InodeType::Dir);
        assert_eq!(event_node.type_(), InodeType::CharDevice);
        assert!(is_kernel_managed(top_dir.as_ref()));
        assert!(is_kernel_managed(input_dir.as_ref()));
        assert!(is_kernel_managed(event_node.as_ref()));

        let metadata = event_node.metadata();
        assert_eq!(metadata.self_dev_id, Some(device_id(240, 1)));
        assert_eq!(metadata.mode.bits(), mkmod!(a+rw).bits());

        delete_node(char_node(path, 240, 1)).unwrap();
        assert_missing("__devtmpfs_ktest_create");
    }

    #[ktest]
    fn delete_node_keeps_nonempty_parent_dir() {
        init();

        let node0 = char_node("__devtmpfs_ktest_nonempty/input/event0", 240, 3);
        let node1 = char_node("__devtmpfs_ktest_nonempty/input/event1", 240, 4);
        create_node(node0).unwrap();
        create_node(node1).unwrap();

        delete_node(char_node("__devtmpfs_ktest_nonempty/input/event0", 240, 3)).unwrap();
        assert_missing("__devtmpfs_ktest_nonempty/input/event0");
        assert!(lookup_relative("__devtmpfs_ktest_nonempty/input").is_ok());
        assert!(lookup_relative("__devtmpfs_ktest_nonempty/input/event1").is_ok());

        delete_node(char_node("__devtmpfs_ktest_nonempty/input/event1", 240, 4)).unwrap();
        assert_missing("__devtmpfs_ktest_nonempty");
    }

    #[ktest]
    fn delete_node_keeps_unmarked_node() {
        init();

        let path = "__devtmpfs_ktest_unmarked";
        let root = root_inode();
        root.mknod(
            path,
            mkmod!(a+rw),
            MknodType::CharDevice(device_id(240, 5).as_encoded_u64()),
        )
        .unwrap();
        let inode = root.lookup(path).unwrap();
        assert!(!is_kernel_managed(inode.as_ref()));

        delete_node(char_node(path, 240, 5)).unwrap();

        let inode = root.lookup(path).unwrap();
        assert!(!is_kernel_managed(inode.as_ref()));
        root.unlink(path).unwrap();
    }

    #[ktest]
    fn delete_node_keeps_device_id_mismatch() {
        init();

        let path = "__devtmpfs_ktest_mismatch";
        create_node(char_node(path, 240, 6)).unwrap();

        delete_node(char_node(path, 240, 7)).unwrap();

        let inode = lookup_relative(path).unwrap();
        assert!(is_kernel_managed(inode.as_ref()));
        assert_eq!(inode.metadata().self_dev_id, Some(device_id(240, 6)));
        delete_node(char_node(path, 240, 6)).unwrap();
    }
}
