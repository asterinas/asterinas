use crate::fs::exfat::fat::ExfatChain;

use crate::fs::exfat::fs::ExfatFS;
use crate::fs::utils::{Inode, InodeType};

use super::dentry::ExfatDentry;
use core::time::Duration;
use jinux_frame::vm::VmFrame;
use jinux_rights::Full;
use crate::fs::utils::InodeMode;
use crate::fs::utils::DirentVisitor;
use crate::events::IoEvents;
use crate::process::signal::Poller;
use crate::vm::vmo::Vmo;
use crate::fs::device::Device;
use alloc::string::String;
use crate::fs::utils::IoctlCmd;
use crate::prelude::*;
//In-memory rust object that represents a file or folder.
#[derive(Default,Debug,Clone)]
pub struct ExfatInode{
    dir: ExfatChain,
    entry: i32,
    type_: u8,  
    attr: u16,
    start_cluster: u32,
    flags: u8,
    version: u32,

    i_size_ondisk: u64,
    i_size_aligned: u64,
    i_pos: u64,

    //Usefor for folders
    num_subdirs: u32,

    atime: Duration,
    mtime: Duration,
    ctime: Duration,

    //exFAT uses UTF-16 encoding, rust use utf-8 for string processing.
    //namebuf: ExfatDentryNameBuf,
    //TODO: should use weak ptr
    fs: Weak<ExfatFS>

    // hint_bmap: ExfatHint,
    // hint_stat: ExfatHint,
    // hint_femp: ExfatHintFemp,

    // cache_lru_lock: SpinLock,
    // cache_lru: ListHead,
    // nr_caches: i32,
    // cache_valid_id: u32,

    
    // i_hash_fat: HlistNode,
    // truncate_lock: RwSemaphore,
    
    // vfs_inode: Inode,
    // i_crtime: Timespec,
}


impl TryFrom<&[ExfatDentry]> for ExfatInode {
    type Error = crate::error::Error;
    fn try_from(dentries: &[ExfatDentry]) -> Result<Self>{
        unimplemented!()
        // let mut ret:ExfatInode;
        // //dentry 0 must be file/dir dentry
        // if let ExfatDentry::File(dentry) = dentries[0] {
        //     ret.type_ = dentry.dentry_type;
        //     ret.attr = dentry.attribute;
        //     //TODO: handle time conversion from DOS format.
        //     todo!("Implement time conversion");
        //     todo!("Read Name buf");
        //     todo!("Read Stream dentry")
        // } else {
        //     return_errno_with_message!(Errno::EINVAL,"Not a file dentry")
        // }
        
    }
}



impl Inode for ExfatInode{
    fn len(&self) -> usize {
        todo!()
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
        todo!()
    }

    fn set_atime(&self, time: core::time::Duration) {
        todo!()
    }

    fn mtime(&self) -> core::time::Duration {
        todo!()
    }

    fn set_mtime(&self, time: core::time::Duration) {
        todo!()
    }

    fn fs(&self) -> alloc::sync::Arc<dyn crate::fs::utils::FileSystem> {
        todo!()
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