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

pub trait FatTrait{
    fn get_next_fat(&self,cluster:u32) -> Result<FatValue>;
    fn set_next_fat(&self,cluster:u32,value:FatValue) -> Result<()>;
    fn find_free_fat(&self,start_cluster:u32, end_cluster:u32) -> u32;
    fn count_free_fat(&self,end_cluster:u32) -> Result<u32>;
}

pub struct FatReader<'a>{
    current_cluster : FatValue,
    fs: &'a ExfatFS
}

impl Iterator for FatReader<'a>{
    type Item = &[u8];

    fn next(&mut self) -> Option<Self::Item>;
}

impl FatTrait for ExfatFS {
    fn is_valid_cluster(&self, cluster:u32) ->bool{
        return cluster >= EXFAT_FIRST_CLUSTER && cluster < self.super_block().num_clusters;
    }
    fn get_fat(&self,cluster:u32) -> Result<FatValue>{
        let sb : ExfatSuperBlock = self.super_block();
        let sector_size = sb.sector_size;

        if !self.is_valid_cluster(cluster) {
            return_errno_with_message!(Errno::EIO,&format!("invalid access to FAT (entry 0x{})",cluster))
        }
        
        let position = u64(sb.fat1_start_sector) * sector_size + u64(cluster) * FAT_ENT_SIZE;
        let mut buf : [u8;size_of::<u32>()];
        self.block_device().read_at(position, &mut buf)?;

        let value = u32::from_le_bytes(buf);
        return Ok(FatValue::from(value));
    }

    fn set_fat(&self,cluster:u32,value:FatValue) -> Result<()> {
        let sb : ExfatSuperBlock = self.super_block();
        let sector_size = sb.sector_size;

        let position = u64(sb.fat1_start_sector) * sector_size + u64(cluster) * FAT_ENT_SIZE;
        let raw_value: u32 = value.into();
        
        //TODO: should make sure that the write is synchronous.
        self.block_device().write_at(position, &raw_value.to_le_bytes())?;

        
        if sb.fat1_start_sector != sb.fat2_start_sector {
            let mirror_position = u64(sb.fat2_start_sector) * sector_size + u64(cluster) * FAT_ENT_SIZE;
            self.block_device().write_at(mirror_position, &raw_value.to_le_bytes())?;
        }

       Ok(())
    }
}



