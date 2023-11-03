use std::ascii::AsciiExt;

use crate::prelude::*;

use super::{fs::ExfatFS, dentry::{ExfatDentryIterator, ExfatDentry, ExfatBitmapDentry}, fat::ExfatChain, constants::ALLOC_FAT_CHAIN};

pub struct ExfatBitmap<'a>{
    /// start cluster of allocation bitmap
    map_cluster:u32,
    // TODO: Import bitvec library
    bitvec:BitVec,
    fs:Weak<ExfatFS>
}

impl ExfatBitmap{
    pub fn load_bitmap(fs:&ExfatFS) -> Result<Self> {
        let exfat_dentry_iterator = ExfatDentryIterator::from(fs,0,ExfatChain{
            dir:fs.super_block().root_dir,
            size:0,
            flags:ALLOC_FAT_CHAIN
        });


        for dentry_result in exfat_dentry_iterator{
            let dentry = dentry_result?;
            if let ExfatDentry::Bitmap(bitmap_dentry) = dentry {
                if bitmap_dentry.flags == 0 {
                    Self::allocate_bitmap(fs,&bitmap_dentry)
                }
            }
        }

        return_errno!(Errno::EINVAL)
    }

    fn allocate_bitmap(fs:&ExfatFS,dentry:&ExfatBitmapDentry) -> Result<Self> {

        let mut buf = vec![0;dentry.size];
        let bitmap_bytes = fs.block_device().read_at(fs.cluster_to_off(dentry.start_cluster), &mut buf);
        let result = ExfatBitmap{
            map_cluster:dentry.start_cluster,
            bitvec:BitVec::from_bytes(bitmap_bytes),
            fs:Weak::from(fs)
        };
    }

    pub fn set_bitmap(&self,cluster:u32, sync:bool) {
        todo!()
        //TODO:Write the bitmap to disk 
    }

    pub fn clear_bitmap(&self,cluster:u32, sync:bool) {
        todo!()
        //TODO:Write the bitmap to disk 
    }

    //Return the first free cluster
    pub fn find_free_bitmap(&self,cluster:u32) -> u32{
        todo!()
    }


}