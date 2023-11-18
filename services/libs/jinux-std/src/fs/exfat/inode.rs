use crate::fs::exfat::dentry::read_name_from_dentry_set;
use crate::fs::exfat::fat::ExfatChain;

use crate::fs::exfat::fs::ExfatFS;
use crate::fs::exfat::utils::make_mode;
use crate::fs::utils::{Inode, InodeType, PageCache, Metadata};

use super::block_device::is_block_aligned;
use super::constants::*;
use super::dentry::{ExfatDentry, update_checksum_for_dentry_set};
use super::utils::{convert_dos_time_to_duration, convert_duration_to_dos_time};
use super::fat::{FatValue, FatTrait};
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
use jinux_frame::vm::{VmFrame, VmIo, VmAllocOptions, VmFrameVec};
use jinux_rights::Full;
pub(super) use align_ext::AlignExt;

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

    pub(super) fn is_name_valid(&self) -> bool{
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
#[derive(Debug)]
pub struct ExfatInode {
    ino:    usize,

    parent_dir: ExfatChain,
    parent_entry: u32,

    type_: u16,

    attr: u16,
    start_cluster: u32,

    flags: u8,
    version: u32,

    //Logical size
    size_on_disk: usize,

    atime: Duration,
    mtime: Duration,
    ctime: Duration,

    num_subdir: u32,

    mode:InodeMode,

    //exFAT uses UTF-16 encoding, rust use utf-8 for string processing.
    name: ExfatDentryName,

    page_cache:PageCache,

    fs: Weak<ExfatFS>
}

impl ExfatInode {

    pub(super) fn new() -> Arc<Self> {
        let inode = Arc::new_cyclic(|weak_self| Self {
            ino:0,
            parent_dir: ExfatChain::default(),
            parent_entry: 0,
            type_: 0,
            attr: 0,
            size_on_disk:0,
            start_cluster:0,
            flags: 0,
            version: 0,
            atime:Duration::default(),
            mtime:Duration::default(),
            ctime:Duration::default(),
            num_subdir:0,
            mode:InodeMode::empty(),
            name:ExfatDentryName::default(),
            fs: Weak::default(),
            page_cache:PageCache::new(weak_self.clone() as _).unwrap()
        });
        inode
    }
    pub fn fs(&self) -> Arc<ExfatFS> {
        self.fs.upgrade().unwrap()
    }

    /* Return the FAT attribute byte for this inode */
    fn make_attr(&self) -> u16 {
        let mut attr = self.attr;
        if self.type_ == TYPE_DIR {
            attr |= ATTR_SUBDIR;
        }

        if self.mode_can_hold_read_only() && (self.mode().bits() & 0o222) == 0{
            attr |= ATTR_READONLY;
        }
        return attr;
    }

    /*
    * If ->i_mode can't hold 0222 (i.e. ATTR_RO), we use ->i_attrs to
    * save ATTR_RO instead of ->i_mode.
    *
    * If it's directory and !sbi->options.rodir, ATTR_RO isn't read-only
    *  bit, it's just used as flag for app.
    */
    fn mode_can_hold_read_only(&self) -> bool{
        if self.type_ == TYPE_DIR {
            return false;
        }
        if ((!self.fs().mount_option().fs_fmask) & 0o222) != 0 {
            return true;
        }
        return false;
    }

    fn write_inode(&self, sync:bool) -> Result<()>{

        //If the inode is already unlinked, there is no need for updating it.
        if self.parent_dir.dir == DIR_DELETED {
            return Ok(());
        }

        if self.type_ == TYPE_DIR && self.parent_dir.dir == self.fs().super_block().root_dir {
            return Ok(());
        }

        //Need to read the latest dentry set.
        let mut dentries = self.fs().get_dentry_set(&self.parent_dir,self.parent_entry, ES_ALL_ENTRIES)?;

        if let ExfatDentry::File(mut file_dentry) = dentries[ES_IDX_FILE] {
            if let ExfatDentry::Stream(mut stream_dentry) = dentries[ES_IDX_STREAM] {
                
                file_dentry.attribute = self.make_attr();

                let create_time = convert_duration_to_dos_time(self.ctime);
                file_dentry.create_tz = create_time.0;
                file_dentry.create_date = create_time.1;
                file_dentry.create_time = create_time.2;
                file_dentry.create_time_cs = create_time.3;

                let modify_time = convert_duration_to_dos_time(self.mtime);
                file_dentry.modify_tz = modify_time.0;
                file_dentry.modify_date = modify_time.1;
                file_dentry.modify_time = modify_time.2;
                file_dentry.modify_time_cs = modify_time.3;

                let access_time = convert_duration_to_dos_time(self.atime);
                file_dentry.access_tz = access_time.0;
                file_dentry.access_date = access_time.1;
                file_dentry.access_time = access_time.2;
               

                let mut size_on_disk = self.size_on_disk;
                if self.start_cluster == EXFAT_EOF_CLUSTER {
                    size_on_disk = 0;
                }

                stream_dentry.valid_size = size_on_disk as u64;
                stream_dentry.size = stream_dentry.valid_size;

                if size_on_disk != 0 {
                    stream_dentry.flags = self.flags;
                    stream_dentry.start_cluster = self.start_cluster;
                } else {
                    stream_dentry.flags = ALLOC_FAT_CHAIN;
                    stream_dentry.start_cluster = EXFAT_FREE_CLUSTER;
                }     

                dentries[ES_IDX_FILE] = ExfatDentry::File(file_dentry);
                dentries[ES_IDX_STREAM] = ExfatDentry::Stream(stream_dentry);

                update_checksum_for_dentry_set(&mut dentries);
                return self.fs().put_dentry_set(&dentries, &self.parent_dir, self.parent_entry, sync);
                
            } else {
                return_errno_with_message!(Errno::EINVAL,"not a file dentry")
            }
        } else {
            return_errno_with_message!(Errno::EINVAL,"not a stream dentry")
        }
        
    }

    //Get the sector id from the logical sector id in the inode.
    fn get_or_allocate_sector_id(&self,sector_id:usize,create:bool,alloc_page:bool) -> Result<usize>{
        let sector_size = self.fs().super_block().sector_size as usize;

        let sector_per_page = PAGE_SIZE / sector_size;
        let mut sector_end = sector_id;
        if alloc_page {
            sector_end = sector_id.align_up(sector_per_page);
        }


        let last_sector = self.size_on_disk / sector_size;

        if sector_end >= last_sector && !create {
            return_errno!(Errno::EINVAL)
        }

        //Cluster size must be larger than page_size.
        let cluster = self.get_or_allocate_cluster_on_disk(sector_end as u32 >> self.fs().super_block().sect_per_cluster_bits,create)?;

        if cluster == EXFAT_EOF_CLUSTER {
            //FIXME: What error code should I return.
            return_errno!(Errno::EINVAL)
        }

        let sec_offset = sector_id % (self.fs().super_block().sect_per_cluster as usize);

        Ok(self.fs().cluster_to_off(cluster) / sector_size + sec_offset)
    }

    // allocate clusters in fat mode, return the first allocated cluster id
    fn alloc_cluster_fat(&self, num_to_be_allocated:u32, sync_bitmap:bool) -> Result<u32> {
        let fs = self.fs.upgrade().unwrap();
        let bitmap = fs.bitmap().upgrade().unwrap();
        let sb = fs.super_block();
        let mut alloc_start_cluster = 0;
        let mut prev_cluster = 0;
        let mut cur_cluster = EXFAT_FIRST_CLUSTER;
        for i in 0..num_to_be_allocated {
            cur_cluster = bitmap.find_next_free_cluster(cur_cluster)?;
            //bitmap.set_bitmap_used(cur_cluster, sync_bitmap);
            if i == 0 {
                alloc_start_cluster = cur_cluster;
            }
            else {
                fs.set_next_fat(prev_cluster, FatValue::Data(cur_cluster))?;
            }
            prev_cluster = cur_cluster;
        }
        fs.set_next_fat(prev_cluster, FatValue::EndOfChain)?;
        Ok(alloc_start_cluster)
    }


    //Get the cluster id from the logical cluster id in the inode.
    //exFAT do not support holes in the file, so new clusters need to be allocated.
    fn get_or_allocate_cluster_on_disk(&self,cluster:u32,create:bool) -> Result<u32> {
        let fs = self.fs.upgrade().unwrap();
        let sb = fs.super_block();
        let cur_cluster_num = (self.size_on_disk + sb.cluster_size as usize - 1) >> sb.cluster_size_bits;
        if cluster >= cur_cluster_num as u32 {
            if create {
                self.alloc_cluster(cluster - cur_cluster_num as u32 + 1, true)?;
            }
            else {
                return_errno_with_message!(Errno::EINVAL, "invaild cluster id");
            }
        }
        let mut cluster_id = self.start_cluster;
        if self.flags == ALLOC_POSSIBLE|ALLOC_NO_FAT_CHAIN {
            cluster_id += cluster;
        }
        else {
            for i in 0..cluster {
                let fat_value =  fs.get_next_fat(cluster_id)?;
                match fat_value {
                    FatValue::Data(new_id) => { cluster_id = new_id; }
                    _ => { return_errno_with_message!(Errno::EINVAL, "invaild fat entry"); }
                }
            }
        }
        Ok(cluster_id)
    }

    // append clusters at the end of file, return the first allocated cluster
    fn alloc_cluster(&self,num_to_be_allocated:u32,sync_bitmap:bool) -> Result<u32> {
        let fs = self.fs.upgrade().unwrap();
        let bitmap = fs.bitmap().upgrade().unwrap();
        let sb = fs.super_block();
        let cur_cluster_num = (self.size_on_disk + sb.cluster_size as usize - 1) >> sb.cluster_size_bits;
        let no_fat_chain: bool = self.flags == ALLOC_POSSIBLE|ALLOC_NO_FAT_CHAIN;
        let mut alloc_start_cluster: u32;

        // if current capacity is 0(no start_cluster), this means we can choose a allocation type
        // first try continuous allocation
        // if no continuous available, turn to fat allocation
        if cur_cluster_num == 0 {
            // search for a continuous chunk big enough
            let search_result = bitmap.find_next_free_cluster_chunk_opt(EXFAT_FIRST_CLUSTER, num_to_be_allocated);
            match search_result {
                Result::Ok(start_cluster) => {
                    alloc_start_cluster = start_cluster;
                    //bitmap.set_bitmap_used_chunk(start_cluster, num_to_be_allocated, sync_bitmap);
                    //self.start_cluster = alloc_start_cluster;
                    //self.flags = ALLOC_POSSIBLE|ALLOC_NO_FAT_CHAIN;
                    return Ok(alloc_start_cluster);
                }
                _ => {
                    // no continuous chunk available, use fat table
                    alloc_start_cluster = self.alloc_cluster_fat(num_to_be_allocated, sync_bitmap)?;
                    //self.start_cluster = alloc_start_cluster;
                    //self.flags = ALLOC_POSSIBLE|ALLOC_FAT_CHAIN;
                    return Ok(alloc_start_cluster);
                }
            }
        }
        // append the exist clusters
        let mut trans_from_no_fat = false;
        if no_fat_chain {
            // first, check for if there are enough following clusters.
            // if not, we can give up continuous allocation and turn to fat allocation
            alloc_start_cluster = self.start_cluster + cur_cluster_num as u32;
            let ok_to_append = bitmap.is_cluster_chunk_free(alloc_start_cluster, num_to_be_allocated)?;
            if ok_to_append {
                //bitmap.set_bitmap_used_chunk(alloc_start_cluster, num_to_be_allocated, sync_bitmap);
                return Ok(alloc_start_cluster);
            }
            else {
                // fill in fat for already allocated part
                for i in 0..cur_cluster_num - 1 {
                    fs.set_next_fat(self.start_cluster + i as u32, FatValue::Data(self.start_cluster + i as u32 + 1))?;
                }
                fs.set_next_fat(self.start_cluster + cur_cluster_num  as u32 - 1, FatValue::EndOfChain)?;
                //self.flags = ALLOC_POSSIBLE|ALLOC_FAT_CHAIN;
                trans_from_no_fat = true;
            }
        }
        // fat allocation (2 possibilty: originally fat allocation; transformed from continuous allocation)
        let cur_end_cluster: u32;
        if trans_from_no_fat {
            cur_end_cluster = self.start_cluster + cur_cluster_num as u32 - 1;
        }
        else {
            cur_end_cluster = self.get_or_allocate_cluster_on_disk(cur_cluster_num as u32 - 1, false)?;
        }
        alloc_start_cluster = self.alloc_cluster_fat(num_to_be_allocated, sync_bitmap)?;
        fs.set_next_fat(cur_end_cluster, FatValue::Data(alloc_start_cluster))?;
        return Ok(alloc_start_cluster);
    }


    //The caller of the function should give a unique ino to assign to the inode.
    pub(super) fn read_inode(fs: Arc<ExfatFS>, p_dir: ExfatChain, entry: u32, ino: usize) -> Result<Arc<Self>> {
        let dentry_set = fs.get_dentry_set(&p_dir, entry, ES_ALL_ENTRIES)?;
        if let ExfatDentry::File(file) = dentry_set[ES_IDX_FILE] {
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

            

            if dentry_set.len() < EXFAT_FILE_MIMIMUM_DENTRY {
                return_errno_with_message!(Errno::EINVAL, "Invalid dentry length")
            }

            //STREAM Dentry must immediately follows file dentry
            if let ExfatDentry::Stream(stream) = dentry_set[ES_IDX_STREAM] {
                let size_on_disk = stream.valid_size as usize;

                let start_cluster = stream.start_cluster;
                let flags = stream.flags;

                
                //Read name from dentry
                let name = read_name_from_dentry_set(&dentry_set);
                if !name.is_name_valid() {
                    return_errno_with_message!(Errno::EINVAL, "Invalid name")
                }

                let mode = make_mode(fs.mount_option(),InodeMode::from_bits(0o777).unwrap(),file.attribute);

                let num_subdir = 0;
                if type_ == TYPE_DIR {
                    //TODO: count subdirs.
                }

                let fs_weak = Arc::downgrade(&fs);

                return Ok(Arc::new_cyclic(|weak_self|ExfatInode {
                    ino,
                    parent_dir: p_dir,
                    parent_entry: entry,
                    type_: type_,
                    attr: file.attribute,
                    size_on_disk,
                    start_cluster,
                    flags: flags,
                    version: 0,
                    atime,
                    mtime,
                    ctime,
                    num_subdir,
                    mode,
                    name,
                    fs: fs_weak,
                    page_cache:PageCache::new(weak_self.clone() as _).unwrap()
                }));
            } else {
                return_errno_with_message!(Errno::EINVAL, "Invalid stream dentry")
            }
        }
        return_errno_with_message!(Errno::EINVAL,"Invalid file dentry")
    }

    
    pub fn resize(&self,new_size:usize) -> Result<()> {
        let no_fat_chain: bool = self.flags == ALLOC_POSSIBLE|ALLOC_NO_FAT_CHAIN;
        let fs = self.fs.upgrade().unwrap();
        let bitmap = fs.bitmap().upgrade().unwrap();
        let sb = fs.super_block();
        let cur_cluster_num = (self.size_on_disk + sb.cluster_size as usize - 1) >> sb.cluster_size_bits;
        let need_cluster_num= (new_size + sb.cluster_size as usize - 1) >> sb.cluster_size_bits;
        if need_cluster_num > cur_cluster_num {
            // need to allocate new clusters
            let alloc_num = need_cluster_num - cur_cluster_num;
            self.alloc_cluster(alloc_num as u32, true)?;
        }
        else if need_cluster_num < cur_cluster_num {
            let trunc_num = cur_cluster_num - need_cluster_num;
            // need to truncate exist clusters
            let new_end_cluster = self.get_or_allocate_cluster_on_disk(need_cluster_num as u32 - 1, false)?;
            if no_fat_chain {
                //bitmap.set_bitmap_unused_chunk(new_end_cluster + 1, trunc_num as u32, true)?;
            }
            else {
                let mut prev_cluster = new_end_cluster;
                for i in 0..trunc_num {
                    match fs.get_next_fat(prev_cluster)? {
                        FatValue::Data(cur_cluster) => { 
                            //bitmap.set_bitmap_unused(cur_cluster, true)?;
                            prev_cluster = cur_cluster;
                        }
                        _ => return_errno_with_message!(Errno::EINVAL, "Invalid fat entry")
                    }
                }
                fs.set_next_fat(new_end_cluster, FatValue::EndOfChain)?;
            }
        }
        //self.size_on_disk = new_size;
        Ok(())
    }

    fn page_cache(&self) -> &PageCache {
        &self.page_cache
    }

    fn read_block(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let bid = self.get_or_allocate_sector_id(idx, false,false)?;
        self.fs().block_device().read_block(bid, frame)?;
        Ok(())
    }

    fn write_block(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let bid = self.get_or_allocate_sector_id(idx, true,false)?;
        self.fs().block_device().write_block(bid, frame)?;
        Ok(())
    }

   

    
}

impl Inode for ExfatInode {
    fn len(&self) -> usize {
        self.size_on_disk as usize
    }

    fn resize(&self, new_size: usize) {
        todo!()
    }

    fn metadata(&self) -> crate::fs::utils::Metadata {
        let blk_size = self.fs().super_block().sector_size as usize;
        Metadata{
            dev: 0,
            ino: self.ino,
            size: self.size_on_disk,
            blk_size,
            blocks: (self.size_on_disk + blk_size - 1)/blk_size,
            atime: self.atime(),
            mtime: self.mtime(),
            ctime: self.ctime,
            type_: self.type_(),
            mode: self.mode(),
            nlinks: self.num_subdir as usize,
            uid: self.fs().mount_option().fs_uid,
            gid: self.fs().mount_option().fs_gid,
            //real device
            rdev: 0,
        }
    }

    fn type_(&self) -> InodeType {
        match self.type_{
            TYPE_FILE => InodeType::File,
            TYPE_DIR => InodeType::Dir,
            _ => todo!()
        }
    }

    fn mode(&self) -> InodeMode {
        self.mode
    }

    fn set_mode(&self, mode:InodeMode) {
        todo!("Add write lock");
        //self.mode = make_mode(self.fs().mount_option(), mode, self.attr);
    }

    fn atime(&self) -> Duration {
        self.atime
    }

    fn set_atime(&self, time: Duration) {
        todo!("Add write lock");
        //TODO:Check time
        //self.atime = time;
    }

    fn mtime(&self) -> Duration {
        self.mtime
    }

    fn set_mtime(&self, time: Duration) {
        todo!("Add write lock");
        //TODO:Check time
        //self.mtime = time;
    }

    fn fs(&self) -> alloc::sync::Arc<dyn crate::fs::utils::FileSystem> {
        self.fs()
    }

    //FIXME:When blocksize is not equal to page_size, the function will not work correctly.
    fn read_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let bid = self.get_or_allocate_sector_id(idx, false,true)?;
        self.fs().block_device().read_page(bid, frame)?;
        Ok(())
    }

    //What if block_size is not equal to page size?
    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let bid = self.get_or_allocate_sector_id(idx, true,true)?;
        self.fs().block_device().write_page(bid, frame)?;
        Ok(())
    }


    fn page_cache(&self) -> Option<Vmo<Full>> {
        Some(self.page_cache().pages().dup().unwrap())
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        if self.type_() != InodeType::File {
            return_errno!(Errno::EISDIR)
        }
        let (off,read_len) = {
            let file_size = self.len();
            let start = file_size.min(offset);
            let end = file_size.min(offset + buf.len());
            (start,end - start)
        };
        self.page_cache.pages().read_bytes(offset, &mut buf[..read_len])?;

        Ok(read_len)
    }

     // The offset and the length of buffer must be multiples of the block size.
    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        if self.type_() != InodeType::File {
            return_errno!(Errno::EISDIR);
        }
        if !is_block_aligned(offset) || !is_block_aligned(buf.len()) {
            return_errno_with_message!(Errno::EINVAL, "not block-aligned");
        }

        let block_size = self.fs().super_block().sector_size as usize;

        let (offset, read_len) = {
            let file_size = self.len();
            let start = file_size.min(offset).align_down(block_size);
            let end = file_size.min(offset + buf.len()).align_down(block_size);
            (start, end - start)
        };
        self.page_cache().pages().decommit(offset..offset + read_len)?;

        let mut buf_offset = 0;
        let frame = VmFrameVec::allocate(VmAllocOptions::new(1).uninit(false).can_dma(true)).unwrap().pop().unwrap();

        for bid in offset/block_size..(offset + read_len)/block_size {
            self.read_block(bid, &frame)?;
            frame.read_bytes(0, &mut buf[buf_offset..buf_offset + block_size])?;
            buf_offset += block_size;
        }
        Ok(read_len)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        if self.type_() != InodeType::File {
            return_errno!(Errno::EISDIR)
        }

        let file_size = self.len();
        let new_size = offset + buf.len();
        if new_size > file_size {
            self.page_cache.pages().resize(new_size)?;
        }
        self.page_cache.pages().write_bytes(offset, buf)?;
        if new_size > file_size {
            self.resize(new_size)?;
        }

        Ok(buf.len())
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        if self.type_() != InodeType::File {
            return_errno!(Errno::EISDIR);
        }
        if !is_block_aligned(offset) || !is_block_aligned(buf.len()) {
            return_errno_with_message!(Errno::EINVAL, "not block-aligned");
        }

        let file_size = self.len();
        let end_offset = offset + buf.len();

        let start = offset.min(file_size);
        let end = end_offset.min(file_size);
        self.page_cache.pages().decommit(start..end)?;

        if end_offset > file_size {
            self.page_cache.pages().resize(end_offset)?;
            self.resize(end_offset)?;
        }

        let block_size = self.fs().super_block().sector_size as usize;

        let mut buf_offset = 0;
        for bid in offset/block_size..(end_offset)/block_size  {
            let frame = {
                let frame = VmFrameVec::allocate(VmAllocOptions::new(1).uninit(false).can_dma(true)).unwrap().pop().unwrap();
                frame.write_bytes(0, &buf[buf_offset..buf_offset + block_size])?;
                frame
            };
            self.write_block(bid, &frame)?;
            buf_offset += block_size;
        }

        Ok(buf_offset)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        todo!()
    }

    fn mknod(&self, name: &str, mode: InodeMode, dev: Arc<dyn Device>) -> Result<Arc<dyn Inode>> {
        return_errno_with_message!(Errno::EINVAL,"Unsupported operation")
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR)
        }

        todo!()
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL,"Unsupported operation")
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
        return_errno_with_message!(Errno::EINVAL,"Unsupported operation")
    }

    fn write_link(&self, target: &str) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL,"Unsupported operation")
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        todo!()
    }

    fn sync(&self) -> Result<()> {
        self.page_cache().evict_range(0..self.len())?;
        self.write_inode(true)?;
        Ok(())
    }

    fn poll(&self, mask: IoEvents, _poller: Option<&Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }

    fn is_dentry_cacheable(&self) -> bool {
        true
    }
}
