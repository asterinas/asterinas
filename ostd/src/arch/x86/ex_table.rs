// SPDX-License-Identifier: MPL-2.0

use crate::prelude::Vaddr;

#[repr(C)]
struct ExTableItem {
    inst_addr: Vaddr,
    recovery_inst_addr: Vaddr,
}

extern "C" {
    fn __ex_table();
    fn __ex_table_end();
}

/// A structure representing the usage of exception table (ExTable).
/// This table is used for recovering from specific exception handling faults
/// occurring at known points in the code.
///
/// To add a recovery instruction for a target assembly instruction, one should add
/// the following statements:
///
/// ```
/// .pushsection .ex_table, "a"
/// .align 8
/// .quad [.target_label],
/// .quad [.recovery_label],
/// .popsection
/// ```
///
/// where the `target_label` and `recovery_label` are the labels of the target instruction
/// and the label of recovery instruction respectively.
///
/// For example, we have the following assembly code snippets in an input file:
/// ```
/// .label1:    
///     rep movsb
///     mov rax, rcx
/// .label2:
///     ret
/// ```
///
/// We can add the following statements in the same file (`label1` and `label2` are local
/// labels):
///
/// ```
/// .pushsection .ex_table, "a"
/// .align 8
/// .quad [.label1],
/// .quad [.label2],
/// .popsection
/// ```
///
/// After that, we can use the API of `ExTable` to resume execution when handling
/// exceptions caused by `rep movsb` (which `label1` point to) failing.
pub(crate) struct ExTable;

impl ExTable {
    /// Finds the recovery instruction address for a given instruction address.
    ///
    /// This function is generally used when an exception (such as a page fault) occurs.
    /// if the exception handling fails and there is a predefined recovery action,
    /// then the found recovery action will be taken.
    pub fn find_recovery_inst_addr(inst_addr: Vaddr) -> Option<Vaddr> {
        let table_size =
            (__ex_table_end as usize - __ex_table as usize) / core::mem::size_of::<ExTableItem>();
        // SAFETY: `__ex_table` is a static section consisting of `ExTableItem`.
        let ex_table =
            unsafe { core::slice::from_raw_parts(__ex_table as *const ExTableItem, table_size) };
        for item in ex_table {
            if item.inst_addr == inst_addr {
                return Some(item.recovery_inst_addr);
            }
        }
        None
    }
}
