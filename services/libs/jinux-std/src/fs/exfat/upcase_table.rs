use crate::prelude::*;
use super::{fs::ExfatFS, dentry::{ExfatDentryIterator, ExfatDentry, ExfatUpcaseDentry}, fat::ExfatChain, constants::{ALLOC_FAT_CHAIN, UPCASE_MANDATORY_SIZE, UNICODE_SIZE}};

#[derive(Debug)]
pub struct ExfatUpcaseTable{
    // mapping tabe
    upcase_table: [u16; UPCASE_MANDATORY_SIZE],
    fs:Weak<ExfatFS>
}

impl ExfatUpcaseTable {
    pub fn empty() -> Self{
        Self{
            upcase_table:[0;UPCASE_MANDATORY_SIZE],
            fs:Weak::default()
        }
    }
    pub fn load_upcase_table(fs:Weak<ExfatFS>) -> Result<Self> {
        let root_dir = fs.upgrade().unwrap().super_block().root_dir;
        let exfat_dentry_iterator = ExfatDentryIterator::from(fs.clone(),0,ExfatChain{
            dir:root_dir,
            size:0,
            flags:ALLOC_FAT_CHAIN
        });

        for dentry_result in exfat_dentry_iterator{
            let dentry = dentry_result?;
            if let ExfatDentry::Upcase(upcase_dentry) = dentry {
                return Self::allocate_table(fs,&upcase_dentry);
            }
        }

        return_errno!(Errno::EINVAL)
    }

    fn allocate_table(fs_weak:Weak<ExfatFS>,dentry:&ExfatUpcaseDentry) -> Result<Self> {
        let fs = fs_weak.upgrade().unwrap();
        if (dentry.size as usize) < UPCASE_MANDATORY_SIZE * UNICODE_SIZE {
            return_errno!(Errno::EINVAL)
        }
        let mut buf: Vec<u8> = vec![0;dentry.size as usize];
        fs.block_device().read_at(fs.cluster_to_off(dentry.start_cluster), &mut buf)?;
        
        Self::verify_checksum(&buf, dentry.checksum)?;

        let mut res = ExfatUpcaseTable{
            upcase_table: [0;UPCASE_MANDATORY_SIZE],
            fs:fs_weak
        };
        
        // big endding or small endding? (now small endding)
        for i in 0..UPCASE_MANDATORY_SIZE {
            res.upcase_table[i] = (buf[2 * i] as u16) | ((buf[2 * i + 1] as u16) << 8);
        }

        Ok(res)

        //Ok(ExfatUpcaseTable{
        //    upcase_table: buf[0..UPCASE_MANDATORY_SIZE],
        //    fs
        //})
    }

    fn verify_checksum(data: &Vec<u8>, checksum: u32) -> Result<()> {
        let mut calc: u32 = 0;
        for byte in data {
            calc = (calc << 31) | (calc >> 1) + *byte as u32;
        }
        
        if !(calc == checksum) {
            return_errno!(Errno::EINVAL)
        }
        Ok(())
    }

    pub fn transform_to_upcase(&self, buf: &mut Vec<u16>) -> Result<()> {
        for idx in 0..buf.len(){
            if (buf[idx] as usize) < UPCASE_MANDATORY_SIZE {
                buf[idx] = self.upcase_table[buf[idx] as usize];
            }
        }
        Ok(())
    }
}