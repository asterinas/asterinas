use super::{block_device::BlockDevice, super_block::ExfatSuperBlock, inode::ExfatInode, balloc::ExfatBitmap};

use crate::{fs::{exfat::constants::*, utils::SuperBlock,utils::{FileSystem, Inode}}, return_errno, return_errno_with_message,prelude::*};
use alloc::boxed::Box;

use log::warn;
use super::super_block::ExfatBootSector;
use super::utils::le16_to_cpu;

pub(super) use jinux_frame::vm::VmIo;
use crate::fs::utils::FsFlags;
use crate::fs::exfat::fat::ExfatChain;

#[derive(Debug)]
pub struct ExfatFS{
    block_device: Box<dyn BlockDevice>,
    super_block: ExfatSuperBlock,
    root: Arc<ExfatInode>,
    bitmap: Arc<ExfatBitmap>
    //TODO: Should add a no_std hashmap crate like hashbrown.
    //inode_cache : HashMap<Arc<ExfatInode>>
}

impl ExfatFS{
    pub fn open(block_device:Box<dyn BlockDevice>) -> Result<Arc<Self>> {
        //Load the super_block
        let super_block = Self::read_super_block(block_device.as_ref())?;

        let mut exfat_fs = Arc::new(ExfatFS{
            block_device,
            super_block,
            root:Arc::new(ExfatInode::default()),
            bitmap:Arc::new(ExfatBitmap::default())
        });

        
        // TODO: if the main superblock is corrupted, should we load the backup?
        
        //Verify boot region
        Self::verify_boot_region(exfat_fs.block_device())?;
        
        //TODO: Load Upcase Table

        let bitmap = ExfatBitmap::load_bitmap(Arc::downgrade(&exfat_fs))?;
        let root: ExfatInode = Self::read_root(exfat_fs.block_device())?;

        let fs_mut = Arc::get_mut(&mut exfat_fs).unwrap();
        fs_mut.bitmap = Arc::new(bitmap);
        fs_mut.root = Arc::new(root);

        //TODO: Handle UTF-8

        //TODO: Init Inode Hash Table.

        //TODO: Init NLS Table

        //TODO: Insert root to inode hash table

        Ok(exfat_fs)
    }

    fn read_root(block_device:&dyn BlockDevice ) -> Result<ExfatInode> {
        todo!()
    }

    //TODO: Check boot signature and boot checksum.
    fn verify_boot_region(block_device:& dyn BlockDevice) -> Result<()> {
        todo!()
    }

    fn read_super_block(block_device:& dyn BlockDevice) -> Result<ExfatSuperBlock> {
        let boot_sector = block_device.read_val::<ExfatBootSector>(0)?;
        /* check the validity of BOOT */
        if le16_to_cpu(boot_sector.signature) != BOOT_SIGNATURE {
            return_errno_with_message!(Errno::EINVAL,"invalid boot record signature");
        }
      
        if !boot_sector.fs_name.eq(STR_EXFAT.as_bytes()) {
            return_errno_with_message!(Errno::EINVAL,"invalid fs name");
        }

        /*
	    * must_be_zero field must be filled with zero to prevent mounting
	    * from FAT volume.
	    */
        if boot_sector.must_be_zero.iter().any(|&x| x!=0) {
            return_errno!(Errno::EINVAL);
        }

        if boot_sector.num_fats != 1 && boot_sector.num_fats != 2 {
            return_errno_with_message!(Errno::EINVAL,"bogus number of FAT structure");
        }

        /*
	    * sect_size_bits could be at least 9 and at most 12.
	    */

        //FIXEME: Should I allocate memory for error message?
        if boot_sector.sector_size_bits < EXFAT_MIN_SECT_SIZE_BITS || boot_sector.sector_size_bits > EXFAT_MAX_SECT_SIZE_BITS {
            
            return_errno_with_message!(Errno::EINVAL,"bogus sector size bits");
        }

        if boot_sector.sector_per_cluster_bits > EXFAT_MAX_SECT_SIZE_BITS {
            return_errno_with_message!(Errno::EINVAL,"bogus sector size bits per cluster");
        }

        let super_block = ExfatSuperBlock::try_from(boot_sector)?;

        /* check consistencies */
        if ((super_block.num_fat_sectors as u64) << boot_sector.sector_size_bits) < (super_block.num_clusters as u64) * 4 {
            return_errno_with_message!(Errno::EINVAL,"bogus fat length");
        }

        if super_block.data_start_sector < super_block.fat1_start_sector + (super_block.num_fat_sectors as u64 * boot_sector.num_fats as u64) {
            return_errno_with_message!(Errno::EINVAL,"bogus data start vector");
        }

        if (super_block.vol_flags & VOLUME_DIRTY as u32) != 0 {
            warn!("Volume was not properly unmounted. Some data may be corrupt. Please run fsck.")
        }

        if (super_block.vol_flags & MEDIA_FAILURE as u32 ) != 0 {
            warn!("Medium has reported failures. Some data may be lost.")
        }

        Self::calibrate_blocksize(&super_block,1 << boot_sector.sector_size_bits)?;

        Ok(super_block)


    }

    fn calibrate_blocksize(super_block:&ExfatSuperBlock,logical_sec:u32) -> Result<()> {
        //TODO: logical_sect should be larger than block_size.
        Ok(())
    }

    pub fn block_device(&self) -> &dyn BlockDevice{
        self.block_device.as_ref()
    }

    pub fn super_block(&self) -> ExfatSuperBlock {
        self.super_block
    }

    pub fn root_inode(&self) -> Result<Arc<ExfatInode>> {
        unimplemented!()
    }

    pub fn cluster_to_off(&self,cluster:u32) -> usize{
        (((((cluster - EXFAT_RESERVED_CLUSTERS) as u64) << self.super_block.sect_per_cluster_bits) + self.super_block.data_start_sector)*self.super_block.sector_size as u64) as usize
    }

    pub fn is_valid_cluster(&self,cluster:u32) -> bool {
        cluster >= EXFAT_RESERVED_CLUSTERS && cluster < self.super_block.num_clusters
    }
}


impl FileSystem for ExfatFS {
    fn sync(&self) -> Result<()> {
        // TODO:Sync
        todo!();
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        todo!();
    }

    fn sb(&self) -> SuperBlock {
        unimplemented!()
    }

    fn flags(&self) -> FsFlags {
        FsFlags::DENTRY_UNEVICTABLE
    }
}

    
// Name pub structures
pub struct ExfatUniName {
    name: [u16; MAX_NAME_LENGTH + 3],
    name_hash: u16,
    name_len: u8,
}



//First empty entry hint information
pub struct ExfatHintFemp {
    eidx: i32,
    count: i32,
    cur: ExfatChain,
}

pub struct ExfatHint {
    clu: u32,
    eidx_or_off: u32,
}

// Mount options
// pub struct ExfatMountOptions {
//     fs_uid: uid_t,
//     fs_gid: gid_t,
//     fs_fmask: u16,
//     fs_dmask: u16,
//     allow_utime: u16,
//     iocharset: *mut c_char,
//     errors: ExfatErrorMode,
//     utf8: bool,
//     sys_tz: bool,
//     discard: bool,
//     keep_last_dots: bool,
//     time_offset: i32,
// }

