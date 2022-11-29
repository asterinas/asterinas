use crate::prelude::*;

pub const MAX_PROCESS_NAME_LEN: usize = 128;
pub struct ProcessName {
    inner: [u8; MAX_PROCESS_NAME_LEN],
    count: usize,
}

impl ProcessName {
    pub fn new() -> Self {
        ProcessName {
            inner: [0; MAX_PROCESS_NAME_LEN],
            count: 0,
        }
    }

    pub fn set_name(&mut self, name: &CStr) -> Result<()> {
        let bytes = name.to_bytes_with_nul();
        let bytes_len = bytes.len();
        if bytes_len > MAX_PROCESS_NAME_LEN {
            return_errno_with_message!(Errno::E2BIG, "process name is too long");
        }
        self.count = bytes_len;
        self.inner[..bytes_len].clone_from_slice(bytes);
        Ok(())
    }

    pub fn get_name(&self) -> Result<Option<&CStr>> {
        Ok(Some(&(CStr::from_bytes_with_nul(&self.inner)?)))
    }
}
