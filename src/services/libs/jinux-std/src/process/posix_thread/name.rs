use crate::prelude::*;

pub const MAX_THREAD_NAME_LEN: usize = 16;
pub struct ThreadName {
    inner: [u8; MAX_THREAD_NAME_LEN],
    count: usize,
}

impl ThreadName {
    pub fn new() -> Self {
        ThreadName {
            inner: [0; MAX_THREAD_NAME_LEN],
            count: 0,
        }
    }

    pub fn new_from_executable_path(elf_path: &str) -> Result<Self> {
        let mut thread_name = ThreadName::new();
        let elf_file_name = elf_path
            .split('/')
            .last()
            .ok_or(Error::with_message(Errno::EINVAL, "invalid elf path"))?;
        let name = CString::new(elf_file_name)?;
        thread_name.set_name(&name)?;
        Ok(thread_name)
    }

    pub fn set_name(&mut self, name: &CStr) -> Result<()> {
        let bytes = name.to_bytes_with_nul();
        let bytes_len = bytes.len();
        if bytes_len > MAX_THREAD_NAME_LEN {
            // if len > MAX_THREAD_NAME_LEN, truncate it.
            self.count = MAX_THREAD_NAME_LEN;
            self.inner[..MAX_THREAD_NAME_LEN].clone_from_slice(&bytes[..MAX_THREAD_NAME_LEN]);
            self.inner[MAX_THREAD_NAME_LEN] = 0;
            return Ok(());
        }
        self.count = bytes_len;
        self.inner[..bytes_len].clone_from_slice(bytes);
        Ok(())
    }

    pub fn get_name(&self) -> Result<Option<&CStr>> {
        Ok(Some(&(CStr::from_bytes_with_nul(&self.inner)?)))
    }
}
