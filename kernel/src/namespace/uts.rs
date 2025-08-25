// SPDX-License-Identifier: MPL-2.0

use crate::{
    namespace::UserNamespace,
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::PosixThread},
};

/// The UTS namespace.
pub struct UtsNamespace {
    uts_name: UtsName,
    owner: Arc<UserNamespace>,
}

impl UtsNamespace {
    /// Creates a new UTS namespace.
    pub(super) fn new_init(owner: Arc<UserNamespace>) -> Arc<Self> {
        let copy_slice = |src: &[u8], dst: &mut [u8]| {
            let len = src.len().min(dst.len());
            dst[..len].copy_from_slice(&src[..len]);
        };

        // We intentionally report Linux-like UTS values instead of this OS’s real
        // name and version. These spoofed values satisfy glibc, which inspects
        // uname fields (sysname, release, version, etc.) and expects Linux-compatible data.
        let mut uts_name = UtsName::new();
        copy_slice(b"Linux", &mut uts_name.sysname);
        copy_slice(b"WHITLEY", &mut uts_name.nodename);
        copy_slice(b"5.13.0", &mut uts_name.release);
        copy_slice(b"5.13.0", &mut uts_name.version);
        copy_slice(b"x86_64", &mut uts_name.machine);
        copy_slice(b"", &mut uts_name.domainname);

        Arc::new(Self { uts_name, owner })
    }

    /// Clones a new UTS namespace.
    pub(super) fn clone_new(
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
    pub fn owner(&self) -> &Arc<UserNamespace> {
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

impl UtsName {
    const fn new() -> Self {
        UtsName {
            sysname: [0; UTS_FIELD_LEN],
            nodename: [0; UTS_FIELD_LEN],
            release: [0; UTS_FIELD_LEN],
            version: [0; UTS_FIELD_LEN],
            machine: [0; UTS_FIELD_LEN],
            domainname: [0; UTS_FIELD_LEN],
        }
    }
}
