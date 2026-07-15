// SPDX-License-Identifier: MPL-2.0

use ostd::sync::RwMutexReadGuard;
use spin::Once;

use crate::{
    fs::pseudofs::{NsCommonOps, NsType, StashedDentry},
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
    security::lsm::hooks as lsm_hooks,
};

/// The UTS namespace.
pub struct UtsNamespace {
    uts_name: RwMutex<UtsName>,
    owner: Arc<UserNamespace>,
    stashed_dentry: StashedDentry,
}

impl UtsNamespace {
    /// Returns a reference to the singleton initial UTS namespace.
    pub fn get_init_singleton() -> &'static Arc<UtsNamespace> {
        static INIT: Once<Arc<UtsNamespace>> = Once::new();

        INIT.call_once(|| {
            let uts_name = UtsName {
                sysname: UtsField::from_bytes_until_nul(UtsName::SYSNAME.as_bytes()),
                // Reference: <https://elixir.bootlin.com/linux/v6.16/source/init/Kconfig#L408>.
                nodename: UtsField::from_bytes_until_nul(b"(none)"),
                release: UtsField::from_bytes_until_nul(UtsName::RELEASE.as_bytes()),
                version: UtsField::from_bytes_until_nul(UtsName::VERSION.as_bytes()),
                machine: UtsField::from_bytes_until_nul(UtsName::MACHINE.as_bytes()),
                // Reference: <https://elixir.bootlin.com/linux/v6.16/source/include/linux/uts.h#L17>.
                domainname: UtsField::from_bytes_until_nul(b"(none)"),
            };

            let owner = UserNamespace::get_init_singleton().clone();
            Self::new(uts_name, owner)
        })
    }

    fn new(uts_name: UtsName, owner: Arc<UserNamespace>) -> Arc<Self> {
        let stashed_dentry = StashedDentry::new();
        Arc::new(Self {
            uts_name: RwMutex::new(uts_name),
            owner,
            stashed_dentry,
        })
    }

    /// Clones a new UTS namespace from `self`.
    pub fn new_clone(
        &self,
        owner: Arc<UserNamespace>,
        posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            owner.as_ref(),
            posix_thread,
            CapSet::SYS_ADMIN,
        ))?;
        Ok(Self::new(*self.uts_name.read(), owner))
    }

    /// Returns a read-only lock guard for accessing the UTS name.
    pub fn uts_name(&self) -> RwMutexReadGuard<'_, UtsName> {
        self.uts_name.read()
    }

    /// Sets a new hostname for the UTS namespace.
    ///
    /// This method will fail with `EPERM` if the POSIX thread does not have the
    /// SYS_ADMIN capability in the owner user namespace.
    pub fn set_hostname(&self, new_host_name: UtsField, posix_thread: &PosixThread) -> Result<()> {
        self.check_set_permission(posix_thread)?;
        self.set_hostname_field(new_host_name);
        Ok(())
    }

    /// Sets a new domain name for the UTS namespace.
    ///
    /// This method will fail with `EPERM` if the POSIX thread does not have the
    /// SYS_ADMIN capability in the owner user namespace.
    pub fn set_domainname(
        &self,
        new_domain_name: UtsField,
        posix_thread: &PosixThread,
    ) -> Result<()> {
        self.check_set_permission(posix_thread)?;
        self.set_domainname_field(new_domain_name);
        Ok(())
    }

    fn check_set_permission(&self, posix_thread: &PosixThread) -> Result<()> {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            self.owner.as_ref(),
            posix_thread,
            CapSet::SYS_ADMIN,
        ))
    }

    fn set_hostname_field(&self, new_host_name: UtsField) {
        debug!("set host name: {:?}", new_host_name.as_cstr());
        self.uts_name.write().nodename = new_host_name;
    }

    fn set_domainname_field(&self, new_domain_name: UtsField) {
        debug!("set domain name: {:?}", new_domain_name.as_cstr());
        self.uts_name.write().domainname = new_domain_name;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct UtsName {
    sysname: UtsField,
    nodename: UtsField,
    release: UtsField,
    version: UtsField,
    machine: UtsField,
    domainname: UtsField,
}

impl UtsName {
    // Note that the following names remain constant across all UTS namespaces. They are immutable,
    // meaning the user space cannot change them. They are stored in the UTS namespace for ease of
    // implementing related system calls.
    //
    // In addition, we intentionally report Linux-like UTS values instead of Asterinas' real name
    // and version. These spoofed values satisfy glibc, which inspects uname fields (sysname,
    // release, version, etc.) and expects Linux-compatible data.

    /// The system name.
    pub const SYSNAME: &str = "Linux";

    /// The release name.
    pub const RELEASE: &str = "5.13.0";

    /// The version name.
    pub const VERSION: &str = {
        const BUILD_TIMESTAMP: &str = if let Some(timestamp) = option_env!("ASTER_BUILD_TIMESTAMP")
        {
            timestamp
        } else {
            const UNIX_EPOCH: &str = "Thu Jan  1 00:00:00 UTC 1970";
            UNIX_EPOCH
        };

        // The definition of Linux's UTS_VERSION can be found at:
        // <https://elixir.bootlin.com/linux/v6.18/source/init/Makefile#L37>.
        // Linux specifies that the total length of this string must not exceed 64 bytes.

        // In Linux, the BUILD_VERSION represents the compilation count, which
        // increments each time the kernel is built within the same source tree.
        // We use a fixed value of '1' here to ensure build determinism.
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/scripts/build-version>.
        const BUILD_VERSION: usize = 1;
        const SMP_FLAGS: &str = "SMP ";
        const PREEMPT_FLAGS: &str = "";
        const VERSION: &str =
            const_format::formatcp!("#{BUILD_VERSION} {SMP_FLAGS}{PREEMPT_FLAGS}{BUILD_TIMESTAMP}");
        assert!(VERSION.len() <= UtsField::MAX_BYTES);
        VERSION
    };

    /// The machine name.
    pub const MACHINE: &str = {
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "x86_64")] {
                "x86_64"
            } else if #[cfg(target_arch = "riscv64")] {
                "riscv64"
            } else if #[cfg(target_arch = "loongarch64")] {
                "loongarch64"
            } else if #[cfg(target_arch = "aarch64")] {
                "aarch64"
            } else {
                compile_error!("unsupported target")
            }
        }
    };

    /// Returns the hostname.
    pub fn nodename(&self) -> &UtsField {
        &self.nodename
    }

    /// Returns the NIS domain name.
    pub fn domainname(&self) -> &UtsField {
        &self.domainname
    }
}

/// A nul-terminated UTS field.
///
/// Although this is a POD type, it has a type invariant: a nul byte must be
/// present, and all bytes after the first nul byte must also be nul bytes.
/// Users outside this module must not arbitrarily mutate the content.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct UtsField([u8; UtsField::MAX_BYTES_WITH_NUL]);

impl UtsField {
    /// The maximum byte length of a UTS field, excluding the trailing nul.
    pub const MAX_BYTES: usize = 64;

    /// The storage byte length of a UTS field, including the trailing nul.
    pub const MAX_BYTES_WITH_NUL: usize = Self::MAX_BYTES + 1;

    /// Creates a UTS field from bytes, stopping at the first nul byte.
    ///
    /// If there is no nul byte within the first [`Self::MAX_BYTES`] bytes,
    /// the input is truncated to [`Self::MAX_BYTES`] bytes. The returned field
    /// is always nul-terminated, and all bytes after the first nul byte are
    /// zeroed.
    pub fn from_bytes_until_nul(bytes: &[u8]) -> Self {
        let mut field = [0u8; Self::MAX_BYTES_WITH_NUL];
        let len = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len())
            .min(Self::MAX_BYTES);
        field[..len].copy_from_slice(&bytes[..len]);
        Self(field)
    }

    /// Reads a UTS field from user space.
    pub fn read_from(addr: Vaddr, len: usize, ctx: &Context) -> Result<Self> {
        // UTS fields represent C strings, which must be nul-terminated.
        // Therefore, the user-provided buffer length cannot exceed `Self::MAX_BYTES`
        // to ensure space for the terminating nul byte.
        if len > Self::MAX_BYTES {
            return_errno_with_message!(Errno::EINVAL, "the UTS name is too long");
        }

        let user_space = ctx.user_space();
        let mut reader = user_space.reader(addr, len)?;
        let mut field = [0u8; Self::MAX_BYTES_WITH_NUL];

        // Partial reads are acceptable,
        // but an error is returned if no bytes can be read successfully.
        if let Err((err, 0)) = reader.read_fallible(&mut VmWriter::from(field.as_mut_slice())) {
            return Err(err.into());
        }

        Ok(Self::from_bytes_until_nul(&field))
    }

    /// Returns the UTS field as a C string.
    pub fn as_cstr(&self) -> &CStr {
        CStr::from_bytes_until_nul(self.0.as_bytes()).unwrap()
    }

    /// Returns the underlying byte array.
    pub fn as_array(&self) -> &[u8; Self::MAX_BYTES_WITH_NUL] {
        &self.0
    }
}

impl NsCommonOps for UtsNamespace {
    const TYPE: NsType = NsType::Uts;

    fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        Some(&self.owner)
    }

    fn parent(&self) -> Result<&Arc<Self>> {
        return_errno_with_message!(
            Errno::EINVAL,
            "a UTS namespace does not have a parent namespace"
        );
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
