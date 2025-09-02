// SPDX-License-Identifier: MPL-2.0

use ostd::sync::{WaitQueue, Waiter, Waker};

/// Reason for waking from a signal or timeout capable wait.
pub enum SigTimeoutWake {
    /// Woken by signal delivery.
    Signal,
    /// Woken by timeout expiration.
    Timeout,
}

/// Waker for signal/timeout waits.
pub type SigTimeoutWaker = Waker<SigTimeoutWake>;

/// Waiter for signal/timeout waits.
pub type SigTimeoutWaiter = Waiter<SigTimeoutWake>;

/// Wait queue for signal/timeout scenarios.
pub type SigTimeoutWaitQueue = WaitQueue<SigTimeoutWake>;
