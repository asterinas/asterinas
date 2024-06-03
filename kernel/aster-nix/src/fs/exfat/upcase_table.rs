// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

use align_ext::AlignExt;
use aster_rights::Full;

use super::{
    constants::UNICODE_SIZE,
    dentry::{ExfatDentry, ExfatDentryIterator, ExfatUpcaseDentry, UTF16Char},
    fat::ExfatChain,
    fs::ExfatFS,
    utils::calc_checksum_32,
};
use crate::{fs::exfat::fat::FatChainFlags, prelude::*, vm::vmo::Vmo};

const UPCASE_MANDATORY_SIZE: usize = 128;

#[derive(Debug)]
pub(super) struct ExfatUpcaseTable {
    upcase_table: [u16; UPCASE_MANDATORY_SIZE],
    fs: Weak<ExfatFS>,
}

impl ExfatUpcaseTable {
    pub(super) fn empty() -> Self {
        Self {
            upcase_table: [0; UPCASE_MANDATORY_SIZE],
            fs: Weak::default(),
        }
    }

    pub(super) fn load(
        fs_weak: Weak<ExfatFS>,
        root_page_cache: Vmo<Full>,
        root_chain: ExfatChain,
    ) -> Result<Self> {
        let dentry_iterator = ExfatDentryIterator::new(root_page_cache, 0, None)?;

        for dentry_result in dentry_iterator {
            let dentry = dentry_result?;
            if let ExfatDentry::Upcase(upcase_dentry) = dentry {
                return Self::load_table_from_dentry(fs_weak, &upcase_dentry);
            }
        }

        return_errno_with_message!(Errno::EINVAL, "Upcase table not found")
    }

    fn load_table_from_dentry(fs_weak: Weak<ExfatFS>, dentry: &ExfatUpcaseDentry) -> Result<Self> {
        if (dentry.size as usize) < UPCASE_MANDATORY_SIZE * UNICODE_SIZE {
            return_errno_with_message!(Errno::EINVAL, "Upcase table too small")
        }

        let fs = fs_weak.upgrade().unwrap();
        let num_clusters = (dentry.size as usize).align_up(fs.cluster_size()) / fs.cluster_size();
        let chain = ExfatChain::new(
            fs_weak.clone(),
            dentry.start_cluster,
            Some(num_clusters as u32),
            FatChainFlags::ALLOC_POSSIBLE,
        )?;

        let mut buf = vec![0; dentry.size as usize];
        fs.read_meta_at(chain.physical_cluster_start_offset(), &mut buf)?;

        if dentry.checksum != calc_checksum_32(&buf) {
            return_errno_with_message!(Errno::EINVAL, "invalid checksum")
        }

        let mut res = ExfatUpcaseTable {
            upcase_table: [0; UPCASE_MANDATORY_SIZE],
            fs: fs_weak,
        };

        for i in 0..UPCASE_MANDATORY_SIZE {
            res.upcase_table[i] = (buf[2 * i] as u16) | ((buf[2 * i + 1] as u16) << 8);
        }

        Ok(res)
    }

    pub(super) fn str_to_upcase(&self, value: &str) -> Result<String> {
        // TODO: use upcase table
        Ok(value.to_uppercase())
    }

    pub(super) fn slice_to_upcase(&self, buf: &mut [UTF16Char]) -> Result<()> {
        for value in buf {
            *value = self.char_to_upcase(*value)?;
        }
        Ok(())
    }

    pub(super) fn char_to_upcase(&self, value: UTF16Char) -> Result<UTF16Char> {
        if (value as usize) < UPCASE_MANDATORY_SIZE {
            Ok(self.upcase_table[value as usize])
        } else {
            Ok(value)
        }
    }
}
