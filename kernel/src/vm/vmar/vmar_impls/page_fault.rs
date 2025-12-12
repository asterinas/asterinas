// SPDX-License-Identifier: MPL-2.0

use super::{Interval, RssDelta, Vmar};
use crate::{prelude::*, thread::exception::PageFaultInfo};

impl Vmar {
    pub fn handle_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<()> {
        let inner = self.inner.read();

        let address = page_fault_info.address;
        if let Some(vm_mapping) = inner.vm_mappings.find_one(&address) {
            debug_assert!(vm_mapping.range().contains(&address));

            let mut rss_delta = RssDelta::new(self);
            return vm_mapping.handle_page_fault(&self.vm_space, page_fault_info, &mut rss_delta);
        }

        return_errno_with_message!(
            Errno::EACCES,
            "no VM mappings contain the page fault address"
        );
    }
}
