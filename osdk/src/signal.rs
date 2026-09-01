// SPDX-License-Identifier: MPL-2.0

//! Signal handling for OSDK child processes.
//!
//! This module provides [`SignalGuard`] for observing termination-related
//! signals, along with helpers for interruptible child-process waits.

use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::{Handle as SignalHandle, Signals},
};
use std::{
    process::{Child, ExitStatus},
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use crate::{error::Errno, warn_msg};

const CHILD_WAIT_INTERVAL: Duration = Duration::from_millis(50);

/// A guard that records the first termination-related signal received.
pub(crate) struct SignalGuard {
    /// The received signal number, or zero when no signal has been received.
    signal: Arc<AtomicI32>,
    handle: SignalHandle,
    thread: Option<JoinHandle<()>>,
}

impl SignalGuard {
    /// Installs handlers for `SIGHUP`, `SIGINT`, and `SIGTERM`.
    pub(crate) fn install() -> Result<Self, String> {
        let mut mask = Signals::new([SIGINT, SIGTERM, SIGHUP])
            .map_err(|err| format!("failed to register signal handlers: {err}"))?;
        let signal = Arc::new(AtomicI32::new(0));
        let signal_for_thread = signal.clone();
        let handle = mask.handle();

        let thread = std::thread::spawn(move || {
            if let Some(received_signal) = mask.forever().next() {
                signal_for_thread.store(received_signal, Ordering::SeqCst);
            }
        });

        Ok(Self {
            signal,
            handle,
            thread: Some(thread),
        })
    }

    /// Returns the shared signal state recorded by the guard.
    pub(crate) fn signal(&self) -> &AtomicI32 {
        &self.signal
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

/// Waits for a child process to exit or for a termination-related signal.
pub(crate) fn wait_for_child(child: &mut Child, signal: &AtomicI32) -> Result<ExitStatus, Errno> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(err) => {
                warn_msg!("failed to wait for child process: {err}");
                let _ = child.kill();
                let _ = child.wait();
                return Err(Errno::ExecuteCommand);
            }
        }

        if signal_value(signal).is_some() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Errno::Interrupted);
        }

        std::thread::sleep(CHILD_WAIT_INTERVAL);
    }
}

/// Returns the received signal number, if any.
pub(crate) fn signal_value(signal: &AtomicI32) -> Option<i32> {
    match signal.load(Ordering::SeqCst) {
        0 => None,
        signal => Some(signal),
    }
}
