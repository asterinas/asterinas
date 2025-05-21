// SPDX-License-Identifier: MPL-2.0

use super::Attribute;
use crate::{prelude::*, util::MultiRead};

/// A special type indicates that a segment cannot have attributes.
#[derive(Debug)]
pub enum NoAttr {}

impl Attribute for NoAttr {
    fn type_(&self) -> u16 {
        match *self {}
    }

    fn payload_as_bytes(&self) -> &[u8] {
        match *self {}
    }

    fn read_from(_reader: &mut dyn MultiRead) -> Result<Self>
    where
        Self: Sized,
    {
        return_errno_with_message!(Errno::EINVAL, "`NoAttr` cannot be read");
    }

    fn read_all_from(_reader: &mut dyn MultiRead, _total_len: usize) -> Result<Vec<Self>>
    where
        Self: Sized,
    {
        Ok(Vec::new())
    }
}
