use crate::fs::exfat::fat::ExfatChain;

use crate::fs::exfat::fs::ExfatFS;
use crate::fs::utils::{Inode, InodeType};
use crate::time::now_as_duration;

use super::constants::*;
use super::dentry::ExfatDentry;
use super::utils::{convert_dos_time_to_duration, le16_to_cpu};
use crate::events::IoEvents;
use crate::fs::device::Device;
use crate::fs::utils::DirentVisitor;
use crate::fs::utils::InodeMode;
use crate::fs::utils::IoctlCmd;
use crate::prelude::*;
use crate::process::signal::Poller;
use crate::vm::vmo::Vmo;
use alloc::string::String;
use core::time::Duration;
use jinux_frame::vm::VmFrame;
use jinux_rights::Full;

use crate::time::ClockID;

#[derive(Default, Debug)]
pub struct ExfatDentryName(Vec<u16>);

impl ExfatDentryName {
    pub fn push(&mut self, value: u16) {
        self.0.push(value);
    }

    pub fn from(name: &str) -> Self {
        ExfatDentryName {
            0: name.encode_utf16().collect(),
        }
    }

    pub fn new() -> Self {
        ExfatDentryName { 0: Vec::new() }
    }

    pub fn is_name_valid(&self) -> bool{
        //TODO:verify the name
        true

    }
}

impl ToString for ExfatDentryName {
    fn to_string(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }
}

//In-memory rust object that represents a file or folder.
#[derive(Default, Debug)]
pub struct ExfatInode {
    parent_dir: ExfatChain,
    parent_entry: u32,

    type_: u16,

    attr: u16,
    start_cluster: u32,

    flags: u8,
    version: u32,

    //Logical size
    size: usize,
    //Allocated size
    capacity: usize,

    //Usefor for folders
    //num_subdirs: u32,

    atime: Duration,
    mtime: Duration,
    ctime: Duration,

    //exFAT uses UTF-16 encoding, rust use utf-8 for string processing.
    name: ExfatDentryName,

    fs: Weak<ExfatFS>
}

impl ExfatInode {
    pub fn fs(&self) -> Arc<ExfatFS> {
        self.fs.upgrade().unwrap()
    }

    fn alloc_inode(fs: Arc<ExfatFS>, name: &str, type_: InodeType) -> Result<Arc<Self>> {
        unimplemented!()
    }

    fn lookup_inode(&self, name: &str) {}

    fn iterate_inode(&self) {}

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        unimplemented!();
    }

    fn allocate_size_up_to(&self, offset: usize) -> Result<()> {
        unimplemented!();
    }

    fn update_dentry(&self) -> Result<()> {
        unimplemented!();
    }

    fn write_at(&mut self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        if offset + buf.len() > self.capacity {
            self.allocate_size_up_to(offset + buf.len())?;
        }

        if offset + buf.len() > self.size {
            self.size = offset + buf.len();
        }
        let now = now_as_duration(&ClockID::CLOCK_REALTIME)?;

        self.mtime = now;
        self.atime = now;
        unimplemented!();
    }

    fn read_inode(fs: Arc<ExfatFS>, p_dir: ExfatChain, entry: u32) -> Result<Arc<Self>> {
        let dentry = fs.get_dentry(&p_dir, entry)?;
        if let ExfatDentry::File(file) = dentry {

            let type_ = if (file.attribute & ATTR_SUBDIR) != 0 {
                TYPE_DIR
            } else {
                TYPE_FILE
            };

            let ctime = convert_dos_time_to_duration(
                file.create_tz,
                file.create_date,
                file.create_time,
                file.create_time_cs,
            )?;
            let mtime = convert_dos_time_to_duration(
                file.modify_tz,
                file.modify_date,
                file.modify_time,
                file.modify_time_cs,
            )?;
            let atime = convert_dos_time_to_duration(
                file.access_tz,
                file.access_date,
                file.access_time,
                0,
            )?;

            let dentry_set = fs.get_dentry_set(&p_dir, entry, ES_ALL_ENTRIES)?;

            if dentry_set.len() < EXFAT_FILE_MIMIMUM_DENTRY {
                return_errno_with_message!(Errno::EINVAL, "Invalid dentry length")
            }

            //STREAM Dentry must immediately follows file dentry
            if let ExfatDentry::Stream(stream) = dentry_set[1] {
                let size = stream.valid_size as usize;
                let start_cluster = stream.start_cluster;
                //Read name from dentry
                let name = Self::read_name_from_dentry_set(&dentry_set);
                if !name.is_name_valid() {
                    return_errno_with_message!(Errno::EINVAL, "Invalid name")
                }

                let fs_weak = Arc::downgrade(&fs);

                return Ok(Arc::new(ExfatInode {
                    parent_dir: p_dir,
                    parent_entry: entry,
                    type_: type_,
                    attr: file.attribute,
                    size,
                    start_cluster,
                    flags: 0,
                    version: 0,
                    capacity: 0,
                    atime,
                    mtime,
                    ctime,
                    name,
                    fs: fs_weak,
                }));
            } else {
                return_errno_with_message!(Errno::EINVAL, "Invalid stream dentry")
            }
        }
        return_errno_with_message!(Errno::EINVAL,"Invalid file dentry")
    }

    fn read_name_from_dentry_set(dentry_set: &[ExfatDentry]) -> ExfatDentryName {
        let mut name: ExfatDentryName = ExfatDentryName::new();
        for i in 2..dentry_set.len() {
            if let ExfatDentry::Name(name_dentry) = dentry_set[i] {
                for character in name_dentry.unicode_0_14 {
                    if character == 0 {
                        return name;
                    } else {
                        name.push(le16_to_cpu(character));
                    }
                }
            } else {
                //End of name dentry
                break;
            }
        }
        name
    }


    
}

impl Inode for ExfatInode {
    fn len(&self) -> usize {
        self.size as usize
    }

    fn resize(&self, new_size: usize) {
        todo!()
    }

    fn metadata(&self) -> crate::fs::utils::Metadata {
        todo!()
    }

    fn type_(&self) -> crate::fs::utils::InodeType {
        todo!()
    }

    fn mode(&self) -> crate::fs::utils::InodeMode {
        todo!()
    }

    fn set_mode(&self, mode: crate::fs::utils::InodeMode) {
        todo!()
    }

    fn atime(&self) -> core::time::Duration {
        self.atime
    }

    fn set_atime(&self, time: core::time::Duration) {
        todo!()
    }

    fn mtime(&self) -> core::time::Duration {
        self.mtime
    }

    fn set_mtime(&self, time: core::time::Duration) {
        todo!()
    }

    fn fs(&self) -> alloc::sync::Arc<dyn crate::fs::utils::FileSystem> {
        self.fs()
    }

    fn read_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        todo!()
    }

    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        todo!()
    }

    fn page_cache(&self) -> Option<Vmo<Full>> {
        None
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        todo!()
    }

    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        todo!()
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        todo!()
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        todo!()
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        todo!()
    }

    fn mknod(&self, name: &str, mode: InodeMode, dev: Arc<dyn Device>) -> Result<Arc<dyn Inode>> {
        todo!()
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        todo!()
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        todo!()
    }

    fn unlink(&self, name: &str) -> Result<()> {
        todo!()
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        todo!()
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        todo!()
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        todo!()
    }

    fn read_link(&self) -> Result<String> {
        todo!()
    }

    fn write_link(&self, target: &str) -> Result<()> {
        todo!()
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        todo!()
    }

    fn sync(&self) -> Result<()> {
        todo!()
    }

    fn poll(&self, mask: IoEvents, _poller: Option<&Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }

    /// Returns whether a VFS dentry for this inode should be put into the dentry cache.
    ///
    /// The dentry cache in the VFS layer can accelerate the lookup of inodes. So usually,
    /// it is preferable to use the dentry cache. And thus, the default return value of this method
    /// is `true`.
    ///
    /// But this caching can raise consistency issues in certain use cases. Specifically, the dentry
    /// cache works on the assumption that all FS operations go through the dentry layer first.
    /// This is why the dentry cache can reflect the up-to-date FS state. Yet, this assumption
    /// may be broken. If the inodes of a file system may "disappear" without unlinking through the
    /// VFS layer, then their dentries should not be cached. For example, an inode in procfs
    /// (say, `/proc/1/fd/2`) can "disappear" without notice from the perspective of the dentry cache.
    /// So for such inodes, they are incompatible with the dentry cache. And this method returns `false`.
    ///
    /// Note that if any ancestor directory of an inode has this method returns `false`, then
    /// this inode would not be cached by the dentry cache, even when the method of this
    /// inode returns `true`.
    fn is_dentry_cacheable(&self) -> bool {
        true
    }
}
