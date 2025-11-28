// SPDX-License-Identifier: MPL-2.0

use crate::{fs::fs_resolver::FsResolver, prelude::*};

mod block;
pub(super) mod char;

pub(super) fn init_in_first_kthread() {
    block::init_in_first_kthread();
}

pub(super) fn init_in_first_process(fs_resolver: &FsResolver) -> Result<()> {
    char::init_in_first_process(fs_resolver)?;
    block::init_in_first_process(fs_resolver)?;

    Ok(())
}
