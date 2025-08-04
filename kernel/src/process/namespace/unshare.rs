// SPDX-License-Identifier: MPL-2.0

use ostd::sync::RwArc;

use crate::{prelude::*, process::CloneFlags};

/// Provides administrative APIs for disassociating execution contexts.
pub trait ContextUnshareAdminApi {
    /// Unshares the file table.
    fn unshare_files(&self);
    /// Unshares filesystem attributes.
    fn unshare_fs(&self);
    /// Unshares System V semaphore.
    fn unshare_sysvsem(&self);
    /// Creates and enters new namespaces as specified by the `flags` argument.
    fn unshare_namespaces(&self, flags: CloneFlags) -> Result<()>;
}

impl ContextUnshareAdminApi for Context<'_> {
    fn unshare_files(&self) {
        let mut pthread_file_table = self.posix_thread.file_table().lock();

        let mut thread_local_file_table_ref = self.thread_local.borrow_file_table_mut();
        let thread_local_file_table = thread_local_file_table_ref.unwrap();

        let new_file_table = RwArc::new(thread_local_file_table.read().clone());

        *pthread_file_table = Some(new_file_table.clone_ro());
        *thread_local_file_table = new_file_table;
    }

    fn unshare_fs(&self) {
        let mut fs_ref = self.thread_local.borrow_fs_mut();
        let new_fs = fs_ref.as_ref().clone();
        *fs_ref = Arc::new(new_fs);
    }

    fn unshare_sysvsem(&self) {
        // TODO: Support unsharing System V semaphore.
        warn!("unsharing System V semaphore is not supported");
    }

    fn unshare_namespaces(&self, flags: CloneFlags) -> Result<()> {
        if flags.contains(CloneFlags::CLONE_NEWUSER) {
            return_errno_with_message!(
                Errno::EINVAL,
                "cloning a new user namespace is not supported"
            );
        }

        let user_ns_ref = self.thread_local.borrow_user_ns();

        let mut pthread_ns_proxy = self.posix_thread.ns_proxy().lock();

        let mut thread_local_ns_proxy_ref = self.thread_local.borrow_ns_proxy_mut();
        let thread_local_ns_proxy = thread_local_ns_proxy_ref.unwrap();

        let new_ns_proxy =
            thread_local_ns_proxy.new_clone(&user_ns_ref, flags, self.posix_thread)?;

        *pthread_ns_proxy = Some(new_ns_proxy.clone());
        *thread_local_ns_proxy = new_ns_proxy;

        Ok(())
    }
}
