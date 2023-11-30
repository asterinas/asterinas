use super::{
    constants::{UNICODE_SIZE, UPCASE_MANDATORY_SIZE},
    dentry::{ExfatDentry, ExfatDentryIterator, ExfatUpcaseDentry},
    fat::ExfatChain,
    fs::ExfatFS,
    utils::calc_checksum_32,
};
use crate::{fs::exfat::fat::FatChainFlags, prelude::*};

#[derive(Debug)]
pub struct ExfatUpcaseTable {
    // mapping tabe
    upcase_table: [u16; UPCASE_MANDATORY_SIZE],
    fs: Weak<ExfatFS>,
}

impl ExfatUpcaseTable {
    pub fn empty() -> Self {
        Self {
            upcase_table: [0; UPCASE_MANDATORY_SIZE],
            fs: Weak::default(),
        }
    }

    pub fn load_upcase_table(fs: Weak<ExfatFS>) -> Result<Self> {
        let root_cluster = fs.upgrade().unwrap().super_block().root_dir;
        let chain = ExfatChain::new(fs.clone(), root_cluster, FatChainFlags::ALLOC_POSSIBLE);

        let dentry_iterator = ExfatDentryIterator::new(chain, 0, None)?;

        for dentry_result in dentry_iterator {
            let dentry = dentry_result?;
            if let ExfatDentry::Upcase(upcase_dentry) = dentry {
                return Self::allocate_table(fs, &upcase_dentry);
            }
        }

        return_errno!(Errno::EINVAL)
    }

    fn allocate_table(fs_weak: Weak<ExfatFS>, dentry: &ExfatUpcaseDentry) -> Result<Self> {
        if (dentry.size as usize) < UPCASE_MANDATORY_SIZE * UNICODE_SIZE {
            return_errno!(Errno::EINVAL)
        }

        let chain = ExfatChain::new(
            fs_weak.clone(),
            dentry.start_cluster,
            FatChainFlags::ALLOC_POSSIBLE,
        );
        let mut buf = vec![0; dentry.size as usize];
        chain.read_at(0, &mut buf)?;

        if dentry.checksum != calc_checksum_32(&buf) {
            return_errno_with_message!(Errno::EINVAL, "invalid checksum")
        }

        let mut res = ExfatUpcaseTable {
            upcase_table: [0; UPCASE_MANDATORY_SIZE],
            fs: fs_weak,
        };

        // big endding or small endding? (now small endding)
        for i in 0..UPCASE_MANDATORY_SIZE {
            res.upcase_table[i] = (buf[2 * i] as u16) | ((buf[2 * i + 1] as u16) << 8);
        }

        Ok(res)
    }

    pub fn transform_to_upcase(&self, buf: &mut [u16]) -> Result<()> {
        for value in buf {
            if (*value as usize) < UPCASE_MANDATORY_SIZE {
                *value = self.upcase_table[*value as usize];
            }
        }
        Ok(())
    }
}
