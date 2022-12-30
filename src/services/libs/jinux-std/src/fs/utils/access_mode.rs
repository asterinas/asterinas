use crate::rights::Rights;

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum AccessMode {
    /// read only
    O_RDONLY = 0,
    /// write only
    O_WRONLY = 1,
    /// read write
    O_RDWR = 2,
}

impl AccessMode {
    pub fn is_readable(&self) -> bool {
        match *self {
            AccessMode::O_RDONLY | AccessMode::O_RDWR => true,
            _ => false,
        }
    }

    pub fn is_writable(&self) -> bool {
        match *self {
            AccessMode::O_WRONLY | AccessMode::O_RDWR => true,
            _ => false,
        }
    }
}

impl From<Rights> for AccessMode {
    fn from(rights: Rights) -> AccessMode {
        if rights.contains(Rights::READ) && rights.contains(Rights::WRITE) {
            AccessMode::O_RDWR
        } else if rights.contains(Rights::READ) {
            AccessMode::O_RDONLY
        } else if rights.contains(Rights::WRITE) {
            AccessMode::O_WRONLY
        } else {
            panic!("invalid rights");
        }
    }
}

impl From<AccessMode> for Rights {
    fn from(access_mode: AccessMode) -> Rights {
        match access_mode {
            AccessMode::O_RDONLY => Rights::READ,
            AccessMode::O_WRONLY => Rights::WRITE,
            AccessMode::O_RDWR => Rights::READ | Rights::WRITE,
        }
    }
}
