// SPDX-License-Identifier: MPL-2.0

use ostd::sync::RwMutexReadGuard;
use spin::Once;

use crate::{
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::PosixThread, UserNamespace},
    util::padded,
};

/// The UTS namespace.
pub struct UtsNamespace {
    uts_name: RwMutex<UtsName>,
    owner: Arc<UserNamespace>,
}

impl UtsNamespace {
    /// Returns the Linux machine name for the current architecture.
    const fn machine_name() -> &'static [u8] {
        #[cfg(target_arch = "x86_64")]
        return b"x86_64";
        #[cfg(target_arch = "riscv64")]
        return b"riscv64";
        #[cfg(target_arch = "loongarch64")]
        return b"loongarch64";
        #[cfg(target_arch = "aarch64")]
        return b"aarch64";
        #[cfg(not(any(
            target_arch = "x86_64",
            target_arch = "riscv64",
            target_arch = "loongarch64",
            target_arch = "aarch64"
        )))]
        compile_error!("Unsupported architecture for UTS namespace machine name");
    }

    /// Returns a reference to the singleton initial UTS namespace.
    pub fn get_init_singleton() -> &'static Arc<UtsNamespace> {
        static INIT: Once<Arc<UtsNamespace>> = Once::new();

        INIT.call_once(|| {
            // We intentionally report Linux-like UTS values instead of Asterinas' real
            // name and version. These spoofed values satisfy glibc, which inspects
            // uname fields (sysname, release, version, etc.) and expects Linux-compatible data.
            let version_str = option_env!("OSDK_BUILD_TIMESTAMP").unwrap_or("unknown");
            let uts_name = UtsName {
                sysname: padded(b"Linux"),
                nodename: padded(b"WHITLEY"),
                release: padded(b"5.13.0"),
                version: padded(version_str.as_bytes()),
                machine: padded(Self::machine_name()),
                // Reference: <https://elixir.bootlin.com/linux/v6.16/source/include/linux/uts.h#L17>.
                domainname: padded(b"(none)"),
            };

            let owner = UserNamespace::get_init_singleton().clone();

            Arc::new(Self {
                uts_name: RwMutex::new(uts_name),
                owner,
            })
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
            uts_name: RwMutex::new(*self.uts_name.read()),
            owner,
        }))
    }

    /// Returns the owner user namespace of the namespace.
    pub fn owner_ns(&self) -> &Arc<UserNamespace> {
        &self.owner
    }

    /// Returns a read-only lock guard for accessing the UTS name.
    pub fn uts_name(&self) -> RwMutexReadGuard<'_, UtsName> {
        self.uts_name.read()
    }

    /// Sets a new hostname for the UTS namespace.
    ///
    /// This method will fail with `EPERM` if the caller does not have the SYS_ADMIN capability
    /// in the owner user namespace.
    pub fn set_hostname(&self, addr: Vaddr, len: usize, ctx: &Context) -> Result<()> {
        self.owner.check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;

        let new_host_name = copy_uts_field_from_user(addr, len as _, ctx)?;
        debug!(
            "set host name: {:?}",
            CStr::from_bytes_until_nul(new_host_name.as_bytes()).unwrap()
        );
        self.uts_name.write().nodename = new_host_name;
        Ok(())
    }

    /// Sets a new domain name for the UTS namespace.
    ///
    /// This method will fail with `EPERM` if the caller does not have the SYS_ADMIN capability
    /// in the owner user namespace.
    pub fn set_domainname(&self, addr: Vaddr, len: usize, ctx: &Context) -> Result<()> {
        self.owner.check_cap(CapSet::SYS_ADMIN, ctx.posix_thread)?;

        let new_domain_name = copy_uts_field_from_user(addr, len as _, ctx)?;
        debug!(
            "set domain name: {:?}",
            CStr::from_bytes_until_nul(new_domain_name.as_bytes()).unwrap()
        );
        self.uts_name.write().domainname = new_domain_name;
        Ok(())
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
    /// Returns the system name as UTF-8 string.
    pub fn sysname(&self) -> Result<&str> {
        Self::cstr_bytes_to_str(&self.sysname)
    }

    /// Returns the release name as UTF-8 string.
    pub fn release(&self) -> Result<&str> {
        Self::cstr_bytes_to_str(&self.release)
    }

    /// Returns the version name as UTF-8 string.
    pub fn version(&self) -> Result<&str> {
        Self::cstr_bytes_to_str(&self.version)
    }

    /// Converts a C string bytes to a UTF-8 string.
    fn cstr_bytes_to_str(cstr_bytes: &[u8]) -> Result<&str> {
        CStr::from_bytes_until_nul(cstr_bytes)
            .map_err(|_| Error::with_message(Errno::EINVAL, "not a null-terminated C string"))
            .and_then(|cstr| {
                cstr.to_str()
                    .map_err(|_| Error::with_message(Errno::EINVAL, "not a UTF-8 string"))
            })
    }
}

fn copy_uts_field_from_user(addr: Vaddr, len: u32, ctx: &Context) -> Result<[u8; UTS_FIELD_LEN]> {
    if len.cast_signed() < 0 {
        return_errno_with_message!(Errno::EINVAL, "the buffer length cannot be negative");
    }

    let user_space = ctx.user_space();
    let mut reader = user_space.reader(addr, len as usize)?;

    // UTS fields represent C strings, which must be nul-terminated.
    // Therefore, the user-provided buffer length cannot exceed `UTS_FIELD_LEN - 1`
    // to ensure space for the terminating nul byte.
    if reader.remain() > UTS_FIELD_LEN - 1 {
        return_errno_with_message!(Errno::EINVAL, "the UTS name is too long");
    }

    let mut buffer = [0u8; UTS_FIELD_LEN];

    // Partial reads are acceptable,
    // but an error is returned if no bytes can be read successfully.
    if let Err((err, 0)) = reader.read_fallible(&mut VmWriter::from(buffer.as_mut_slice())) {
        return Err(err.into());
    }

    Ok(buffer)
}
