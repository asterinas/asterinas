use core::mem::size_of;

use super::constants::*;
use super::fs::ExfatFS;
use crate::prelude::*;
use super::super_block::ExfatSuperBlock;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum FatValue {
    Free,
    Data(u32),
    Bad,
    EndOfChain,
}

pub trait FatTrait{
    fn get_next_fat(&self,cluster:u32) -> Result<FatValue>;
    fn set_next_fat(&self,cluster:u32,value:FatValue) -> Result<()>;
    fn find_free_fat(&self,start_cluster:u32, end_cluster:u32) -> Result<u32>;
    fn count_free_fat(&self,end_cluster:u32) -> Result<u32>;
}

impl From<u32> for FatValue{
    fn from(value:u32) -> Self{
        match value{
            EXFAT_BAD_CLUSTER => FatValue::Bad,
            EXFAT_FREE_CLUSTER => FatValue::Free,
            EXFAT_EOF_CLUSTER => FatValue::EndOfChain,
            _ => FatValue::Data(value)
        }
    }
}

impl From<FatValue> for u32 {
    fn from(val: FatValue) -> Self {
        match val{
            FatValue::Free => EXFAT_FREE_CLUSTER,
            FatValue::EndOfChain => EXFAT_EOF_CLUSTER,
            FatValue::Bad => EXFAT_BAD_CLUSTER,
            FatValue::Data(x) => x
        }
    }
}



// Directory pub structures 
#[derive(Default,Debug,Clone)]
pub struct ExfatChain {
    // current cluster number
    pub dir: u32,
    // what about this??? never used???
    pub size: u32,
    // way of addressing(use FAT or not)
    pub flags: u8,
}


impl FatTrait for ExfatFS {
    
    fn get_next_fat(&self,cluster:u32) -> Result<FatValue>{
        let sb : ExfatSuperBlock = self.super_block();
        let sector_size = sb.sector_size;

        if !self.is_valid_cluster(cluster) {
            return_errno_with_message!(Errno::EIO,"invalid access to FAT")
        }
        
        let position = sb.fat1_start_sector * sector_size as u64 + (cluster as u64) * FAT_ENT_SIZE as u64;
        let mut buf : [u8;size_of::<u32>()] =  [0;size_of::<u32>()];
        self.block_device().read_at(position as usize, &mut buf)?;

        let value = u32::from_le_bytes(buf);
        Ok(FatValue::from(value))
    }

    fn set_next_fat(&self,cluster:u32,value:FatValue) -> Result<()> {
        let sb : ExfatSuperBlock = self.super_block();
        let sector_size = sb.sector_size;

        let position = sb.fat1_start_sector * sector_size as u64 + (cluster as u64) * FAT_ENT_SIZE as u64;
        let raw_value: u32 = value.into();
        
        //TODO: should make sure that the write is synchronous.
        self.block_device().write_at(position as usize, &raw_value.to_le_bytes())?;

        
        if sb.fat1_start_sector != sb.fat2_start_sector {
            let mirror_position = sb.fat2_start_sector * sector_size as u64 + (cluster as u64) * FAT_ENT_SIZE as u64;
            self.block_device().write_at(mirror_position as usize, &raw_value.to_le_bytes())?;
        }

       Ok(())
    }

    fn count_free_fat(&self,end_cluster:u32) -> Result<u32> {
        unimplemented!()
    }

    fn find_free_fat(&self,start_cluster:u32, end_cluster:u32) -> Result<u32> {
        unimplemented!()
    }
}



