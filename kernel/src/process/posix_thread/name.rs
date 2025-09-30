// SPDX-License-Identifier: MPL-2.0

use alloc::borrow::ToOwned;

use crate::prelude::*;

pub const MAX_THREAD_NAME_LEN: usize = 16;

#[derive(Debug, Clone)]
pub struct ThreadName([u8; MAX_THREAD_NAME_LEN]);

impl ThreadName {
    fn new() -> Self {
        ThreadName([0; MAX_THREAD_NAME_LEN])
    }

    pub fn new_from_executable_path(executable_path: &str) -> Self {
        let mut thread_name = ThreadName::new();
        let Some(file_name) = executable_path.split('/').next_back() else {
            return thread_name;
        };

        thread_name.set_name_as_bytes(file_name.as_bytes());
        thread_name
    }

    pub fn set_name(&mut self, name: &CStr) {
        self.set_name_as_bytes(name.to_bytes());
    }

    fn set_name_as_bytes(&mut self, name_as_bytes: &[u8]) {
        let name_len = name_as_bytes.len().min(MAX_THREAD_NAME_LEN - 1);
        self.0[..name_len].copy_from_slice(&name_as_bytes[..name_len]);
        self.0[name_len..].fill(0);
    }

    pub fn name(&self) -> &CStr {
        CStr::from_bytes_until_nul(&self.0).unwrap()
    }

    pub fn as_string(&self) -> Option<String> {
        let name = self.name();
        name.to_str().ok().map(|name| name.to_owned())
    }
}
