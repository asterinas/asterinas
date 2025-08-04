// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use crate::{
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::PosixThread, UserNamespace},
    util::padded,
};

/// The UTS namespace.
pub struct UtsNamespace {
    uts_name: UtsName,
    owner: Arc<UserNamespace>,
}

impl UtsNamespace {
    /// Returns a reference to the singleton initial UTS namespace.
    pub fn get_init_singleton() -> &'static Arc<UtsNamespace> {
        static INIT: Once<Arc<UtsNamespace>> = Once::new();

        INIT.call_once(|| {
            // We intentionally report Linux-like UTS values instead of Asterinas' real
            // name and version. These spoofed values satisfy glibc, which inspects
            // uname fields (sysname, release, version, etc.) and expects Linux-compatible data.
            let uts_name = UtsName {
                sysname: padded(b"Linux"),
                nodename: padded(b"WHITLEY"),
                release: padded(b"5.13.0"),
                version: padded(b"5.13.0"),
                machine: padded(b"x86_64"),
                domainname: padded(b""),
            };

            let owner = UserNamespace::get_init_singleton().clone();

            Arc::new(Self { uts_name, owner })
        })
    }

    /// Clones a new UTS namespace from `self`.
    pub fn new_clone(
        &self,
        owner: Arc<UserNamespace>,
        posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        owner.check_cap(CapSet::SYS_ADMIN, posix_thread)?;
        Ok(Arc::new(Self {
            uts_name: self.uts_name,
            owner,
        }))
    }

    /// Returns the owner user namespace of the namespace.
    pub fn owner_ns(&self) -> &Arc<UserNamespace> {
        &self.owner
    }

    /// Returns the UTS name.
    pub fn uts_name(&self) -> &UtsName {
        &self.uts_name
    }
}

const UTS_FIELD_LEN: usize = 65;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct UtsName {
    sysname: [u8; UTS_FIELD_LEN],
    nodename: [u8; UTS_FIELD_LEN],
    release: [u8; UTS_FIELD_LEN],
    version: [u8; UTS_FIELD_LEN],
    machine: [u8; UTS_FIELD_LEN],
    domainname: [u8; UTS_FIELD_LEN],
}
