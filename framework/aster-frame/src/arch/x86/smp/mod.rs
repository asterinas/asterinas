// SPDX-License-Identifier: MPL-2.0

mod boot;

pub(crate) use boot::{
    get_processor_info, init_boot_stack_array, prepare_boot_stacks, send_boot_ipis,
};
