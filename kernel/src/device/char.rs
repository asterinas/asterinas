// SPDX-License-Identifier: MPL-2.0

//! A subsystem for character devices (or char devices for short).

#![expect(dead_code)]

use core::ops::Range;

use device_id::{DeviceId, MajorId};
use inherit_methods_macro::inherit_methods;

use crate::{
    events::IoEvents,
    fs::{
        device::{add_node, Device, DeviceType},
        file_handle::Mappable,
        fs_resolver::FsResolver,
        inode_handle::FileIo,
        utils::{InodeIo, IoctlCmd, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// A character device.
pub trait CharDevice: Send + Sync + Debug {
    /// Returns the name of this char device that should appear in devtmpfs (usually under `/dev`).
    fn devtmpfs_name(&self) -> DevtmpfsName<'_>;

    /// Returns the device ID.
    fn id(&self) -> DeviceId;

    /// Opens the char device, returning a file-like object that the userspace can interact with by doing I/O.
    ///
    /// Multiple calls to this method return the same object (at least logically).
    fn open(&self) -> Result<Arc<dyn FileIo>>;
}

static DEVICE_REGISTRY: Mutex<BTreeMap<u32, Arc<dyn CharDevice>>> = Mutex::new(BTreeMap::new());

/// Registers a new char device.
pub fn register(device: Arc<dyn CharDevice>) -> Result<()> {
    let mut registry = DEVICE_REGISTRY.lock();
    let id = device.id().to_raw();
    if registry.contains_key(&id) {
        return_errno_with_message!(Errno::EEXIST, "char device already exists");
    }
    registry.insert(id, device);

    Ok(())
}

/// Unregisters an existing char device, returning the device if found.
pub fn unregister(id: DeviceId) -> Result<Arc<dyn CharDevice>> {
    DEVICE_REGISTRY
        .lock()
        .remove(&id.to_raw())
        .ok_or(Error::with_message(
            Errno::ENOENT,
            "char device does not exist",
        ))
}

/// Collects all char devices.
pub fn collect_all() -> Vec<Arc<dyn CharDevice>> {
    DEVICE_REGISTRY.lock().values().cloned().collect()
}

/// Looks up a char device of a given device ID.
pub fn lookup(id: DeviceId) -> Option<Arc<dyn CharDevice>> {
    DEVICE_REGISTRY.lock().get(&id.to_raw()).cloned()
}

/// The maximum value of the major device ID of a char device.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/fs/char_dev.c#L104>.
pub const MAX_MAJOR: u16 = 511;

/// The ranges of free char majors.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/linux/fs.h#L2840>.
const DYNAMIC_MAJOR_ID_RANGES: [Range<u16>; 2] = [234..255, 384..512];

static MAJORS: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

/// Acquires a major ID.
///
/// The returned `MajorIdOwner` object represents the ownership to the major ID.
/// Until the object is dropped, this major ID cannot be acquired via `acquire_major` or `allocate_major` again.
pub fn acquire_major(major: MajorId) -> Result<MajorIdOwner> {
    if major.get() > MAX_MAJOR {
        return_errno_with_message!(Errno::EINVAL, "invalid major ID");
    }

    if MAJORS.lock().insert(major.get()) {
        Ok(MajorIdOwner(major))
    } else {
        return_errno_with_message!(Errno::EEXIST, "major ID already acquired")
    }
}

/// Allocates a major ID.
///
/// The returned `MajorIdOwner` object represents the ownership to the major ID.
/// Until the object is dropped, this major ID cannot be acquired via `acquire_major` or `allocate_major` again.
pub fn allocate_major() -> Result<MajorIdOwner> {
    let mut majors = MAJORS.lock();

    for id in DYNAMIC_MAJOR_ID_RANGES
        .iter()
        .flat_map(|range| range.clone().rev())
    {
        if majors.insert(id) {
            return Ok(MajorIdOwner(MajorId::new(id)));
        }
    }

    return_errno_with_message!(Errno::ENOSPC, "no more major IDs available");
}

/// An owned major ID.
///
/// Each instances of this type will unregister the major ID when dropped.
pub struct MajorIdOwner(MajorId);

impl MajorIdOwner {
    /// Returns the major ID.
    pub fn get(&self) -> MajorId {
        self.0
    }
}

impl Drop for MajorIdOwner {
    fn drop(&mut self) {
        MAJORS.lock().remove(&self.0.get());
    }
}

/// A device's name under devtmpfs.
///
/// A `DevtmpfsName` consists of two parts:
/// 1. The device name;
/// 2. The class name.
///
/// # Examples
///
/// If you want a device to appear as `/dev/zero`,
/// then assign it a name of `DevtmpfsName::new("zero", None)`.
///
/// If you want to a device to appear as `/dev/input/event0`,
/// then assign it a name of `DevtmpfsName::new("event0", Some("input"))`.
pub struct DevtmpfsName<'a> {
    dev_name: &'a str,
    class_name: Option<&'a str>,
}

impl<'a> DevtmpfsName<'a> {
    pub fn new(dev_name: &'a str, class_name: Option<&'a str>) -> Self {
        Self {
            dev_name,
            class_name,
        }
    }

    pub fn dev_name(&self) -> &'a str {
        self.dev_name
    }

    pub fn class_name(&self) -> Option<&'a str> {
        self.class_name
    }
}

pub(super) fn init_in_first_process(fs_resolver: &FsResolver) -> Result<()> {
    for device in collect_all() {
        let name = device.devtmpfs_name().dev_name().to_string();
        let device = Arc::new(CharFile::new(device));
        add_node(device, &name, fs_resolver)?;
    }

    Ok(())
}

/// Represents a character device inode in the filesystem.
///
/// Only implements the `Device` trait.
#[derive(Debug)]
pub struct CharFile(Arc<dyn CharDevice>);

impl CharFile {
    pub fn new(device: Arc<dyn CharDevice>) -> Self {
        Self(device)
    }
}

impl Device for CharFile {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.0.id()
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(OpenCharFile(self.0.open()?)))
    }
}

/// Represents an opened character device file ready for I/O operations.
///
/// Does not implement the `Device` trait but provides full implementations
/// for I/O related traits.
pub struct OpenCharFile(Arc<dyn FileIo>);

#[inherit_methods(from = "self.0")]
impl InodeIo for OpenCharFile {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize>;
    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize>;
}

#[inherit_methods(from = "self.0")]
impl Pollable for OpenCharFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}

#[inherit_methods(from = "self.0")]
impl FileIo for OpenCharFile {
    fn check_seekable(&self) -> Result<()>;
    fn is_offset_aware(&self) -> bool;
    fn mappable(&self) -> Result<Mappable>;
    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32>;
}
