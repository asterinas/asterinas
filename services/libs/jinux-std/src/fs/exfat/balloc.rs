use crate::prelude::*;
use bitvec::prelude::BitVec;
use super::{fs::ExfatFS, dentry::{ExfatDentryIterator, ExfatDentry, ExfatBitmapDentry}, fat::ExfatChain, constants::{ALLOC_FAT_CHAIN, EXFAT_RESERVED_CLUSTERS, EXFAT_EOF_CLUSTER}};

pub struct ExfatBitmap<'a>{
    /// start cluster of allocation bitmap
    map_cluster:u32,
    // TODO: use jinux_util::bitmap
    bitvec:BitVec<u8>,
    fs:&'a ExfatFS
}

impl<'a> ExfatBitmap<'a>{
    pub fn load_bitmap(fs:&'a ExfatFS) -> Result<Self> {
        let exfat_dentry_iterator = ExfatDentryIterator::from(fs,0,ExfatChain{
            dir:fs.super_block().root_dir,
            size:0,
            flags:ALLOC_FAT_CHAIN
        });

        for dentry_result in exfat_dentry_iterator{
            let dentry = dentry_result?;
            if let ExfatDentry::Bitmap(bitmap_dentry) = dentry {
                if bitmap_dentry.flags == 0 {
                    return Self::allocate_bitmap(fs,&bitmap_dentry);
                }
            }
        }

        return_errno!(Errno::EINVAL)
    }

    fn allocate_bitmap(fs:&'a ExfatFS,dentry:&ExfatBitmapDentry) -> Result<Self> {

        let mut buf = vec![0;dentry.size as usize];
        fs.block_device().read_at(fs.cluster_to_off(dentry.start_cluster), &mut buf)?;
        
        Ok(ExfatBitmap{
            map_cluster:dentry.start_cluster,
            bitvec:BitVec::from_slice(&buf),
            fs
        })
    }

    pub fn set_bitmap_used(&mut self,cluster:u32, sync:bool) -> Result<()> {
        self.set_bitmap(cluster, true, sync)
    }

    pub fn set_bitmap_unused(&mut self,cluster:u32, sync:bool) -> Result<()> {
        self.set_bitmap(cluster, false, sync)
    }

    fn set_bitmap(&mut self,cluster:u32, bit:bool, sync:bool) -> Result<()> {
        if !self.fs.is_valid_cluster(cluster) {
            return_errno!(Errno::EINVAL)
        }

        let entry_index = cluster - EXFAT_RESERVED_CLUSTERS;
        self.bitvec.set(entry_index as usize, bit);

        self.write_bitmap_byte_to_disk(entry_index, sync)?;
        Ok(())
    }

    fn write_bitmap_byte_to_disk(&self,entry_index:u32, sync:bool) -> Result<()> {
        let byte_off:usize = entry_index as usize / core::mem::size_of::<u8>();
        let bytes:&[u8] = self.bitvec.as_raw_slice();
        let byte = bytes[byte_off];
        
        let byte_off_on_disk = self.fs.cluster_to_off(self.map_cluster) + byte_off;
        
        let _ = self.fs.block_device().write_at(byte_off_on_disk, &[byte]);
        Ok(())
    }

    //Return the first free cluster
    pub fn find_free_bitmap(&self,cluster:u32) -> Result<u32>{
        if !self.fs.is_valid_cluster(cluster) {
            return_errno!(Errno::EINVAL)
        }

        for i in cluster..self.fs.super_block().num_clusters {
            if self.bitvec.get(i as usize).is_none() {
                return Ok(i);
            }
        }

        Ok(EXFAT_EOF_CLUSTER)
    }


}