// SPDX-License-Identifier: MPL-2.0

//! Get and set the current transaction of the current thread.
use core::sync::atomic::Ordering::{Acquire, Release};

use super::{Tx, TxData, TxId, TxProvider, TxStatus};
use crate::{os::CurrentThread, prelude::*};

/// The current transaction on a thread.
#[derive(Clone)]
pub struct CurrentTx<'a> {
    provider: &'a TxProvider,
}

// CurrentTx is only useful and valid for the current thread
impl !Send for CurrentTx<'_> {}
impl !Sync for CurrentTx<'_> {}

impl<'a> CurrentTx<'a> {
    pub(super) fn new(provider: &'a TxProvider) -> Self {
        Self { provider }
    }

    /// Enter the context of the current TX.
    ///
    /// While within the context of a TX, the implementation side of a TX
    /// can get the current TX via `TxProvider::current`.
    pub fn context<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let tx_table = self.provider.tx_table.lock();
        let tid = CurrentThread::id();
        if !tx_table.contains_key(&tid) {
            panic!("there should be one Tx exited on the current thread");
        }

        assert!(tx_table.get(&tid).unwrap().status() == TxStatus::Ongoing);
        drop(tx_table);

        f()
    }

    /// Commits the current TX.
    ///
    /// If the returned value is `Ok`, then the TX is committed successfully.
    /// Otherwise, the TX is aborted.
    pub fn commit(&self) -> Result<()> {
        let mut tx_status = self
            .provider
            .tx_table
            .lock()
            .get(&CurrentThread::id())
            .expect("there should be one Tx exited on the current thread")
            .status();
        debug_assert!(tx_status == TxStatus::Ongoing);

        let res = self.provider.call_precommit_handlers();
        if res.is_ok() {
            self.provider.call_commit_handlers();
            tx_status = TxStatus::Committed;
        } else {
            self.provider.call_abort_handlers();
            tx_status = TxStatus::Aborted;
        }

        let mut tx = self
            .provider
            .tx_table
            .lock()
            .remove(&CurrentThread::id())
            .unwrap();
        tx.set_status(tx_status);
        res
    }

    /// Aborts the current TX.
    pub fn abort(&self) {
        let tx_status = self
            .provider
            .tx_table
            .lock()
            .get(&CurrentThread::id())
            .expect("there should be one Tx exited on the current thread")
            .status();
        debug_assert!(tx_status == TxStatus::Ongoing);

        self.provider.call_abort_handlers();
        let mut tx = self
            .provider
            .tx_table
            .lock()
            .remove(&CurrentThread::id())
            .unwrap();
        tx.set_status(TxStatus::Aborted);
    }

    /// The ID of the transaction.
    pub fn id(&self) -> TxId {
        self.get_current_mut_with(|tx| tx.id())
    }

    /// Get immutable access to some type of the per-transaction data within a closure.
    ///
    /// # Panics
    ///
    /// The `data_with` method must _not_ be called recursively.
    pub fn data_with<T: TxData, F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.get_current_mut_with(|tx| {
            let data = tx.data::<T>();
            f(data)
        })
    }

    /// Get mutable access to some type of the per-transaction data within a closure.
    pub fn data_mut_with<T: TxData, F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        self.get_current_mut_with(|tx| {
            let data = tx.data_mut::<T>();
            f(data)
        })
    }

    /// Get a _mutable_ reference to the current transaction of the current thread,
    /// passing it to a given closure.
    ///
    /// # Panics
    ///
    /// The `get_current_mut_with` method must be called within the closure
    /// of `set_and_exec_with`.
    ///
    /// In addition, the `get_current_mut_with` method must _not_ be called
    /// recursively.
    fn get_current_mut_with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Tx) -> R,
    {
        let mut tx_table = self.provider.tx_table.lock();
        let Some(tx) = tx_table.get_mut(&CurrentThread::id()) else {
            panic!("there should be one Tx exited on the current thread");
        };

        if tx.is_accessing_data.swap(true, Acquire) {
            panic!("get_current_mut_with must not be called recursively");
        }

        let retval: R = f(tx);

        // SAFETY. At any given time, at most one mutable reference will be constructed
        // between the Acquire-Release section. And it is safe to drop `&mut Tx` after
        // `Release`, since drop the reference does nothing to the `Tx` itself.
        tx.is_accessing_data.store(false, Release);

        retval
    }
}
