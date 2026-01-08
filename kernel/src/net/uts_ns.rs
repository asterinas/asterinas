// SPDX-License-Identifier: MPL-2.0

use ostd::{const_assert, sync::RwMutexReadGuard};
use spin::Once;

use crate::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
    util::padded,
};

/// The UTS namespace.
pub struct UtsNamespace {
    uts_name: RwMutex<UtsName>,
    owner: Arc<UserNamespace>,
}

impl UtsNamespace {
    /// Returns a reference to the singleton initial UTS namespace.
    pub fn get_init_singleton() -> &'static Arc<UtsNamespace> {
        static INIT: Once<Arc<UtsNamespace>> = Once::new();

        INIT.call_once(|| {
            let uts_name = UtsName {
                sysname: padded(UtsName::SYSNAME.as_bytes()),
                // Reference: <https://elixir.bootlin.com/linux/v6.16/source/init/Kconfig#L408>.
                nodename: padded(b"(none)"),
                release: padded(UtsName::RELEASE.as_bytes()),
                version: padded(UtsName::VERSION.as_bytes()),
                machine: padded(UtsName::MACHINE.as_bytes()),
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
        const_assert!(VERSION.len() <= 64);
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
}
