use crate::prelude::*;
use bitvec::prelude::*;
use super::{fs::ExfatFS, dentry::{ExfatDentryIterator, ExfatDentry, ExfatBitmapDentry}, fat::ExfatChain, constants::{ALLOC_FAT_CHAIN, EXFAT_RESERVED_CLUSTERS}};


#[derive(Debug,Default)]
pub struct ExfatBitmap{
    /// start cluster of allocation bitmap
    map_cluster:u32,
    // TODO: use jinux_util::bitmap
    bitvec:BitVec<u8>,
    fs:Weak<ExfatFS>
}

impl ExfatBitmap{
    
    pub fn load_bitmap(fs:Weak<ExfatFS>) -> Result<Self> {
        let root_dir = fs.upgrade().unwrap().super_block().root_dir;
        let exfat_dentry_iterator = ExfatDentryIterator::from(fs.clone(),0,ExfatChain{
            dir:root_dir,
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

    fn fs(&self) -> Arc<ExfatFS> {
        self.fs.upgrade().unwrap()
    }

    fn allocate_bitmap(fs:Weak<ExfatFS>,dentry:&ExfatBitmapDentry) -> Result<Self> {

        let mut buf = vec![0;dentry.size as usize];
        fs.upgrade().unwrap().block_device().read_at(fs.upgrade().unwrap().cluster_to_off(dentry.start_cluster), &mut buf)?;
        
        Ok(ExfatBitmap{
            map_cluster:dentry.start_cluster,
            bitvec:BitVec::from_slice(&buf),
            fs
        })
    }

    pub fn set_bitmap_used(&mut self,cluster:u32, sync:bool) -> Result<()> {
        self.set_bitmap_chunk(cluster, 1, true, sync)
    }

    pub fn set_bitmap_unused(&mut self,cluster:u32, sync:bool) -> Result<()> {
        self.set_bitmap_chunk(cluster, 1, false, sync)
    }
    
    pub fn set_bitmap_used_chunk(&mut self, start_cluster:u32, cluster_num:u32, sync:bool) -> Result<()> {
        self.set_bitmap_chunk(start_cluster, cluster_num, true, sync)
    }

    pub fn set_bitmap_unused_chunk(&mut self, start_cluster:u32, cluster_num:u32, sync:bool) -> Result<()> {
        self.set_bitmap_chunk(start_cluster, cluster_num, false, sync)
    }

    pub fn is_cluster_free(&self, cluster:u32) -> Result<bool> {
        self.is_cluster_chunk_free(cluster, 1)
    }

    pub fn is_cluster_chunk_free(&self, start_cluster:u32, cluster_num:u32) -> Result<bool> {
        if !self.fs().is_valid_cluster_chunk(start_cluster, cluster_num) {
            return_errno!(Errno::EINVAL)
        }
        
        let start_index = start_cluster - EXFAT_RESERVED_CLUSTERS;
        let mut is_free = true;
        for id in start_index..start_index + cluster_num {
            if self.bitvec[id as usize] {
                is_free = false;
                break;
            }
        }
        Ok(is_free)
    }

    //Return the first free cluster
    pub fn find_next_free_cluster(&self,cluster:u32) -> Result<u32>{
        self.find_next_free_cluster_chunk(cluster, 1)
    }

    //Return the first free cluster chunk, set cluster_num=1 to find a single cluster
    pub fn find_next_free_cluster_chunk(&self, start_cluster:u32, cluster_num:u32) -> Result<u32> {
        if !self.fs().is_valid_cluster_chunk(start_cluster, cluster_num) {
            return_errno!(Errno::EINVAL)
        }

        let mut cur_index = start_cluster - EXFAT_RESERVED_CLUSTERS;
        let end_index = self.fs().super_block().num_clusters;
        let search_end_index = end_index - cluster_num + 1;
        let mut start_index: u32 = 0;
        let mut found = false;
        while cur_index < search_end_index {
            if self.bitvec.get(cur_index as usize).is_none() {
                start_index  = cur_index;
                let mut cnt = 0;
                while cnt < cluster_num && cur_index < end_index && self.bitvec.get(cur_index as usize).is_none() {
                    cnt += 1;
                    cur_index += 1;
                }
                if cnt >= cluster_num {
                    found = true;
                    break;
                }
            }
            cur_index += 1;
        }

        if found {
            Ok(start_index + EXFAT_RESERVED_CLUSTERS)
        }
        else {
            return_errno!(Errno::EINVAL)
        }
    }

    pub fn find_next_free_cluster_chunk_opt(&self, start_cluster:u32, cluster_num:u32) -> Result<u32> {
        if !self.fs().is_valid_cluster_chunk(start_cluster, cluster_num) {
            return_errno!(Errno::EINVAL)
        }

        let bytes:&[u8] = self.bitvec.as_raw_slice();
        let unit_size: u32 = 8;
        let start_cluster_index = start_cluster - EXFAT_RESERVED_CLUSTERS;
        let mut cur_unit_index = start_cluster_index / unit_size;
        let mut cur_unit_offset = start_cluster_index % unit_size;
        let total_cluster_num = self.fs().super_block().num_clusters;
        let complete_unit_num = total_cluster_num / unit_size;
        let rest_cluster_num = total_cluster_num % unit_size;
        let mut head_cluster_num;
        let mut mid_unit_num;
        let mut tail_cluster_num;
        let mut found: bool = false;
        let mut result_bit_index = 0;
        if cluster_num > unit_size {
            // treat a continuous bit chunk as lead_bits+mid_units+tail_bits
            // mid_units are unit aligned
            // for example: 11110000 00000000 00000000 00111111
            //                  **** -------- -------- ..
            //                  ^(start bit)
            // (*): head_bits;  (-): mid_units;  (.): tail_bits 
            // the start bit can be identified with a pair (cur_unit_index, cur_unit_offset)
            while cur_unit_index < complete_unit_num {
                found = true;
                head_cluster_num = unit_size - cur_unit_offset;
                mid_unit_num = (cluster_num - head_cluster_num) / unit_size;
                tail_cluster_num = (cluster_num - head_cluster_num) % unit_size;
                
                // if the last complete unit to be checked is out of range, stop searching
                if cur_unit_index + mid_unit_num >= complete_unit_num {
                    found = false;
                    break;
                }

                // first, check for the head bits
                let leading_zeros = bytes[cur_unit_index as usize].leading_zeros();
                if head_cluster_num > leading_zeros {
                    cur_unit_offset = unit_size - leading_zeros;
                    if cur_unit_offset == unit_size {
                        cur_unit_index += 1;
                        cur_unit_offset = 0;
                    }
                    found = false;
                    continue;
                }

                // then check for the mid units, these units should be all zero
                // due to previous check, there will be no array out of bounds situation
                for i in 1..mid_unit_num + 1 {
                    if bytes[(cur_unit_index + i) as usize] != 0{
                        cur_unit_index += i;
                        cur_unit_offset = unit_size - bytes[(cur_unit_index + i) as usize].leading_zeros();
                        if cur_unit_offset == unit_size {
                            cur_unit_index += 1;
                            cur_unit_offset = 0;
                        }
                        found = false;
                        break;
                    }
                }

                if !found {
                    continue;
                }

                // at last, check for the tail bits
                let mut tail_byte: u8 = 0;
                if cur_unit_index + mid_unit_num + 1 == complete_unit_num {
                    // for the tail part, there are two special cases: 
                    //      1. this part is out of range;
                    //      2. this part exists, but are partly invaild;
                    if rest_cluster_num == 0 {
                        // in this case, the tail part is out of range
                        found = tail_cluster_num == 0;
                        result_bit_index = cur_unit_index * unit_size + cur_unit_offset;
                        break;
                    }
                    else {
                        // in this case, the tail unit isn't a complete unit, we should set the invaild part of this unit to 1
                        // the invaild part <=> high (unit_size - rest_cluster_num) bits of tail unit
                        tail_byte |= 0xFF as u8 - (((1 as u8) << rest_cluster_num) - 1);
                    }
                    
                }
                tail_byte |= bytes[(cur_unit_index + mid_unit_num + 1) as usize];
                let tailing_zeros = tail_byte.trailing_zeros();
                if tail_cluster_num > tailing_zeros {
                    cur_unit_index = cur_unit_index + mid_unit_num + 1;
                    cur_unit_offset = tailing_zeros + 1;
                    if cur_unit_offset == unit_size {
                        cur_unit_index += 1;
                        cur_unit_offset = 0;
                    }
                    found = false;
                    continue;
                }

                // if we reach here, it means we have found a result
                result_bit_index = cur_unit_index * unit_size + cur_unit_offset;
                break;
            }
            if found {
                Ok(result_bit_index + EXFAT_RESERVED_CLUSTERS)
            }
            else {
                return_errno!(Errno::EINVAL)
            }
        }
        else {
            // cluster_num <= unit_size, back to the simple function
            self.find_next_free_cluster_chunk(start_cluster, cluster_num)
        }
    }


    /* 
    fn set_bitmap(&mut self,cluster:u32, bit:bool, sync:bool) -> Result<()> {
        if !self.fs().is_valid_cluster(cluster) {
            return_errno!(Errno::EINVAL)
        }

        let entry_index = cluster - EXFAT_RESERVED_CLUSTERS;
        self.bitvec.set(entry_index as usize, bit);

        self.write_bitmap_byte_to_disk(entry_index, sync)?;
        Ok(())
    }
    */

    fn set_bitmap_chunk(&mut self, start_cluster:u32, cluster_num:u32, bit:bool, sync:bool) -> Result<()> {
        if !self.fs().is_valid_cluster_chunk(start_cluster, cluster_num) {
            return_errno!(Errno::EINVAL)
        }

        let start_index = start_cluster - EXFAT_RESERVED_CLUSTERS;
        for i in 0..cluster_num {
            self.bitvec.set((start_index + i) as usize, bit);
        }

        self.write_bitmap_chunk_to_disk(start_cluster, cluster_num, sync)?;
        Ok(())
    }

    /* 
    fn write_bitmap_byte_to_disk(&self,entry_index:u32, sync:bool) -> Result<()> {
        let byte_off:usize = entry_index as usize / core::mem::size_of::<u8>();
        let bytes:&[u8] = self.bitvec.as_raw_slice();
        let byte = bytes[byte_off];
        
        let byte_off_on_disk = self.fs().cluster_to_off(self.map_cluster) + byte_off;
        
        let _ = self.fs().block_device().write_at(byte_off_on_disk, &[byte]);
        Ok(())
    }
    */

    fn write_bitmap_chunk_to_disk(&self, start_index:u32, cluster_num:u32, sync:bool) -> Result<()> {
        let start_byte_off:usize = start_index as usize / core::mem::size_of::<u8>();
        let end_byte_off:usize = start_byte_off + cluster_num as usize - 1  / core::mem::size_of::<u8>();
        let bytes:&[u8] = self.bitvec.as_raw_slice();
        let byte_chunk = &bytes[start_byte_off..end_byte_off + 1];
        
        let byte_off_on_disk = self.fs().cluster_to_off(self.map_cluster) + start_byte_off;
        
        let _ = self.fs().block_device().write_at(byte_off_on_disk, byte_chunk);
        Ok(())
    }


}