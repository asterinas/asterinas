// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use crate::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
};

const PRIVILEGED_PORTS: Range<u16> = 0..1024;

/// Checks if the port is privileged and, if so, whether the thread is allowed to bind to it.
pub fn check_port_privilege(port: u16) -> Result<()> {
    if !PRIVILEGED_PORTS.contains(&port) {
        return Ok(());
    }

    // This should be checked under the network namespace's owner user namespace, if we later
    // support those namespaces.
    let thread = current_thread!();
    let posix_thread = thread.as_posix_thread().unwrap();
    if lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::NET_BIND_SERVICE,
    ))
    .is_ok()
    {
        return Ok(());
    }

    return_errno_with_message!(
        Errno::EACCES,
        "only threads with CAP_NET_BIND_SERVICE can bind to privileged ports"
    );
}
