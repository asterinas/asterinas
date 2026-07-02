// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use align_ext::AlignExt;
use ostd::{mm::PAGE_SIZE, task::disable_preempt};

use super::Vmar;
use crate::{
    error::Error,
    vm::vmar::{
        cursor_util::{check_range_mapped, find_next_mapped},
        interval_set::Interval,
        vm_mapping::VmMapping,
    },
};

impl Vmar {
    /// Calls the provided function for each mapping in the VMAR.
    pub fn for_each_mapping(
        &self,
        range: Range<usize>,
        check_fully_mapped: bool,
        mut f: impl FnMut(&VmMapping),
    ) -> Result<(), Error> {
        let preempt_guard = disable_preempt();
        let vm_space = self.vm_space();
        let range_aligned = range.start.align_down(PAGE_SIZE)..range.end.align_up(PAGE_SIZE);
        let mut cursor = vm_space.cursor(&preempt_guard, &range_aligned).unwrap();

        if check_fully_mapped {
            check_range_mapped!(&mut cursor, range_aligned.end)?;
        }

        while let Some(vm_mapping) = find_next_mapped!(cursor, range_aligned.end) {
            let vm_mapping_end = vm_mapping.range().end;

            f(vm_mapping);

            if cursor.jump(vm_mapping_end).is_err() {
                break;
            }
        }

        Ok(())
    }
}
