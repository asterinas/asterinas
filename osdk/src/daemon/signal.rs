// SPDX-License-Identifier: MPL-2.0

//! Signal handling for OSDK child processes.
//!
//! This module provides [`SignalGuard`], which records the first
//! termination-related signal received by OSDK. The first signal starts the
//! normal cleanup path; later signals are passed to their default handlers so
//! a stuck cleanup can still terminate OSDK.

use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::{Handle as SignalHandle, Signals},
    low_level::emulate_default_handler,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    },
    thread::JoinHandle,
};

use crate::{error::Errno, error_msg};

/// A guard that records the first termination-related signal received.
///
/// OSDK handles the first signal itself so it can stop QEMU and its managed
/// daemons. Subsequent signals restore their default behavior and are allowed
/// to terminate OSDK if that cleanup does not make progress.
pub(crate) struct SignalGuard {
    /// The received signal number, or zero when no signal has been received.
    signal: Arc<AtomicI32>,
    handle: SignalHandle,
    thread: Option<JoinHandle<()>>,
}

impl SignalGuard {
    /// Installs handlers for `SIGHUP`, `SIGINT`, and `SIGTERM`.
    pub(crate) fn install() -> Result<Self, Errno> {
        let mut mask = Signals::new([SIGINT, SIGTERM, SIGHUP]).map_err(|err| {
            error_msg!("failed to register signal handlers: {err}");
            Errno::ExecuteCommand
        })?;
        let signal = Arc::new(AtomicI32::new(0));
        let signal_for_thread = signal.clone();
        let handle = mask.handle();

        let thread = std::thread::spawn(move || {
            // Keep observing signals for the guard's whole lifetime. The first
            // signal requests orderly cleanup; a repeated signal falls through
            // to the default disposition instead of being swallowed.
            let mut first_signal = true;
            for received_signal in mask.forever() {
                if !first_signal {
                    let _ = emulate_default_handler(received_signal);
                    continue;
                }
                signal_for_thread.store(received_signal, Ordering::SeqCst);
                first_signal = false;
            }
        });

        Ok(Self {
            signal,
            handle,
            thread: Some(thread),
        })
    }

    /// Returns the received signal number, if any.
    pub(crate) fn received_signal(&self) -> Option<i32> {
        match self.signal.load(Ordering::SeqCst) {
            0 => None,
            signal => Some(signal),
        }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
