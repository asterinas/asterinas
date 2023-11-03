use std::fs::FileTimes;

use crate::fs::utils::{Inode, InodeType};
use crate::fs::exfat::fs;
use crate::return_errno_with_message;
use super::dentry::ExfatDentry;
use core::time::Duration;

//In-memory rust object that represents a file or folder.
#[derive(Default,Debug,Clone)]
pub struct ExfatInode<'a>{
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
    namebuf: ExfatDentryNameBuf,
    //TODO: should use weak ptr
    fs: &'a ExfatFS

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
    fn try_from(dentries: &[ExfatDentry]) -> Result<Self>{
        let ret:ExfatInode;
        //dentry 0 must be file/dir dentry
        if let ExfatDentry::File(dentry) = dentries[0] {
            ret.type_ = dentry.dentry_type
        } else {
            return_errno_with_message!(Errno::EINVAL,"Not a file dentry")
        }
        //dentry 1 must be stream
        //dentry 2 
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
        Err(Error::new(Errno::EISDIR))
    }

    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn page_cache(&self) -> Option<Vmo<Full>> {
        None
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn mknod(&self, name: &str, mode: InodeMode, dev: Arc<dyn Device>) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn unlink(&self, name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn read_link(&self) -> Result<String> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_link(&self, target: &str) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        Err(Error::new(Errno::EISDIR))
    }

    fn sync(&self) -> Result<()> {
        Ok(())
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