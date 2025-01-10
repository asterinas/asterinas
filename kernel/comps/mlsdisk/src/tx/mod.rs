// SPDX-License-Identifier: MPL-2.0

//! Transaction management.
//!
//! Transaction management APIs serve two sides:
//!
//! * The user side of TXs uses `Tx` to use, commit, or abort TXs.
//! * The implementation side of TXs uses `TxProvider` to get notified
//!   when TXs are created, committed, or aborted by register callbacks.
mod current;

use core::{
    any::{Any, TypeId},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

pub use self::current::CurrentTx;
use crate::{
    os::{CurrentThread, HashMap, Mutex, RwLock, Tid},
    prelude::*,
};

/// A transaction provider.
#[expect(clippy::type_complexity)]
pub struct TxProvider {
    id: u64,
    initializer_map: RwLock<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
    precommit_handlers: RwLock<Vec<Box<dyn Fn(CurrentTx<'_>) -> Result<()> + Send + Sync>>>,
    commit_handlers: RwLock<Vec<Box<dyn Fn(CurrentTx<'_>) + Send + Sync>>>,
    abort_handlers: RwLock<Vec<Box<dyn Fn(CurrentTx<'_>) + Send + Sync>>>,
    weak_self: Weak<Self>,
    tx_table: Mutex<HashMap<Tid, Tx>>,
}

impl TxProvider {
    /// Creates a new TX provider.
    pub fn new() -> Arc<Self> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        Arc::new_cyclic(|weak_self| Self {
            id: NEXT_ID.fetch_add(1, Ordering::Release),
            initializer_map: RwLock::new(HashMap::new()),
            precommit_handlers: RwLock::new(Vec::new()),
            commit_handlers: RwLock::new(Vec::new()),
            abort_handlers: RwLock::new(Vec::new()),
            weak_self: weak_self.clone(),
            tx_table: Mutex::new(HashMap::new()),
        })
    }

    /// Creates a new TX that is attached to this TX provider.
    pub fn new_tx(&self) -> CurrentTx<'_> {
        let mut tx_table = self.tx_table.lock();
        let tid = CurrentThread::id();
        if tx_table.contains_key(&tid) {
            return self.current();
        }

        let tx = Tx::new(self.weak_self.clone());
        let _ = tx_table.insert(tid, tx);
        self.current()
    }

    /// Get the current TX.
    ///
    /// # Panics
    ///
    /// The caller of this method must be within the closure passed to
    /// `Tx::context`. Otherwise, the method would panic.
    pub fn current(&self) -> CurrentTx<'_> {
        CurrentTx::new(self)
    }

    /// Register a per-TX data initializer.
    ///
    /// The registered initializer function will be called upon the creation of
    /// a TX.
    pub fn register_data_initializer<T>(&self, f: Box<dyn Fn() -> T + Send + Sync>)
    where
        T: TxData,
    {
        let mut initializer_map = self.initializer_map.write();
        initializer_map.insert(TypeId::of::<T>(), Box::new(f));
    }

    fn init_data<T>(&self) -> T
    where
        T: TxData,
    {
        let initializer_map = self.initializer_map.read();
        let init_fn = initializer_map
            .get(&TypeId::of::<T>())
            .unwrap()
            .downcast_ref::<Box<dyn Fn() -> T + Send + Sync>>()
            .unwrap();
        init_fn()
    }

    /// Register a callback for the pre-commit stage,
    /// which is before the commit stage.
    ///
    /// Committing a TX triggers the pre-commit stage as well as the commit
    /// stage of the TX.
    /// On the pre-commit stage, the register callbacks will be called.
    /// Pre-commit callbacks are allowed to fail (unlike commit callbacks).
    /// If any pre-commit callbacks failed, the TX would be aborted and
    /// the commit callbacks would not get called.
    pub fn register_precommit_handler<F>(&self, f: F)
    where
        F: Fn(CurrentTx<'_>) -> Result<()> + Send + Sync + 'static,
    {
        let f = Box::new(f);
        let mut precommit_handlers = self.precommit_handlers.write();
        precommit_handlers.push(f);
    }

    fn call_precommit_handlers(&self) -> Result<()> {
        let current = self.current();
        let precommit_handlers = self.precommit_handlers.read();
        for precommit_func in precommit_handlers.iter().rev() {
            precommit_func(current.clone())?;
        }
        Ok(())
    }

    /// Register a callback for the commit stage,
    /// which is after the pre-commit stage.
    ///
    /// Committing a TX triggers first the pre-commit stage of the TX and then
    /// the commit stage. The callbacks for the commit stage is not allowed
    /// to fail.
    pub fn register_commit_handler<F>(&self, f: F)
    where
        F: Fn(CurrentTx<'_>) + Send + Sync + 'static,
    {
        let f = Box::new(f);
        let mut commit_handlers = self.commit_handlers.write();
        commit_handlers.push(f);
    }

    fn call_commit_handlers(&self) {
        let current = self.current();
        let commit_handlers = self.commit_handlers.read();
        for commit_func in commit_handlers.iter().rev() {
            commit_func(current.clone())
        }
    }

    /// Register a callback for the abort stage.
    ///
    /// A TX enters the abort stage when the TX is aborted by the user
    /// (via `Tx::abort`) or by a callback in the pre-commit stage.
    pub fn register_abort_handler<F>(&self, f: F)
    where
        F: Fn(CurrentTx<'_>) + Send + Sync + 'static,
    {
        let f = Box::new(f);
        let mut abort_handlers = self.abort_handlers.write();
        abort_handlers.push(f);
    }

    fn call_abort_handlers(&self) {
        let current = self.current();
        let abort_handlers = self.abort_handlers.read();
        for abort_func in abort_handlers.iter().rev() {
            abort_func(current.clone())
        }
    }
}

/// A transaction.
pub struct Tx {
    id: TxId,
    provider: Weak<TxProvider>,
    data_map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    status: TxStatus,
    is_accessing_data: AtomicBool,
}

impl Tx {
    fn new(provider: Weak<TxProvider>) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);

        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Release),
            provider,
            data_map: HashMap::new(),
            status: TxStatus::Ongoing,
            is_accessing_data: AtomicBool::new(false),
        }
    }

    /// Returns the TX ID.
    pub fn id(&self) -> TxId {
        self.id
    }

    /// Returns the status of the TX.
    pub fn status(&self) -> TxStatus {
        self.status
    }

    /// Sets the status of the Tx.
    pub fn set_status(&mut self, status: TxStatus) {
        self.status = status;
    }

    fn provider(&self) -> Arc<TxProvider> {
        self.provider.upgrade().unwrap()
    }

    fn data<T>(&mut self) -> &T
    where
        T: TxData,
    {
        self.data_mut::<T>()
    }

    fn data_mut<T>(&mut self) -> &mut T
    where
        T: TxData,
    {
        let exists = self.data_map.contains_key(&TypeId::of::<T>());
        if !exists {
            // Slow path, need to initialize the data
            let provider = self.provider();
            let data: T = provider.init_data::<T>();
            self.data_map.insert(TypeId::of::<T>(), Box::new(data));
        }

        // Fast path
        self.data_map
            .get_mut(&TypeId::of::<T>())
            .unwrap()
            .downcast_mut::<T>()
            .unwrap()
    }
}

impl Drop for Tx {
    fn drop(&mut self) {
        assert!(
            self.status() != TxStatus::Ongoing,
            "transactions must be committed or aborted explicitly"
        );
    }
}

/// The status of a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatus {
    Ongoing,
    Committed,
    Aborted,
}

/// The ID of a transaction.
pub type TxId = u64;

/// Per-transaction data.
///
/// Using `TxProvider::register_data_initiailzer` to inject per-transaction data
/// and using `CurrentTx::data_with` or `CurrentTx::data_mut_with` to access
/// per-transaction data.
pub trait TxData: Any + Send + Sync {}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeSet;

    use super::*;

    /// `Db<T>` is a toy implementation of in-memory database for
    /// a set of items of type `T`.
    ///
    /// The most interesting feature of `Db<T>` is the support
    /// of transactions. All queries and insertions to the database must
    /// be performed within transactions. These transactions ensure
    /// the atomicity of insertions even in the presence of concurrent execution.
    /// If transactions are aborted, their changes won't take effect.
    ///
    /// The main limitation of `Db<T>` is that it only supports
    /// querying and inserting items, but not deleting.
    /// The lack of support of deletions rules out the possibilities
    /// of concurrent transactions conflicting with each other.
    pub struct Db<T> {
        all_items: Arc<Mutex<BTreeSet<T>>>,
        tx_provider: Arc<TxProvider>,
    }

    struct DbUpdate<T> {
        new_items: BTreeSet<T>,
    }

    impl<T: 'static> TxData for DbUpdate<T> {}

    impl<T> Db<T>
    where
        T: Ord + 'static,
    {
        /// Creates an empty database.
        pub fn new() -> Self {
            let new_self = Self {
                all_items: Arc::new(Mutex::new(BTreeSet::new())),
                tx_provider: TxProvider::new(),
            };

            new_self
                .tx_provider
                .register_data_initializer(Box::new(|| DbUpdate {
                    new_items: BTreeSet::<T>::new(),
                }));
            new_self.tx_provider.register_commit_handler({
                let all_items = new_self.all_items.clone();
                move |mut current: CurrentTx<'_>| {
                    current.data_mut_with(|update: &mut DbUpdate<T>| {
                        let mut all_items = all_items.lock();
                        all_items.append(&mut update.new_items);
                    });
                }
            });

            new_self
        }

        /// Creates a new DB transaction.
        pub fn new_tx(&self) -> CurrentTx<'_> {
            self.tx_provider.new_tx()
        }

        /// Returns whether an item is contained.
        ///
        /// # Transaction
        ///
        /// This method must be called within the context of a transaction.
        pub fn contains(&self, item: &T) -> bool {
            let is_new_item = {
                let current_tx = self.tx_provider.current();
                current_tx.data_with(|update: &DbUpdate<T>| update.new_items.contains(item))
            };
            if is_new_item {
                return true;
            }

            let all_items = self.all_items.lock();
            all_items.contains(item)
        }

        /// Inserts a new item into the DB.
        ///
        /// # Transaction
        ///
        /// This method must be called within the context of a transaction.
        pub fn insert(&self, item: T) {
            let all_items = self.all_items.lock();
            if all_items.contains(&item) {
                return;
            }

            let mut current_tx = self.tx_provider.current();
            current_tx.data_mut_with(|update: &mut DbUpdate<_>| {
                update.new_items.insert(item);
            });
        }

        /// Collects all items of the DB.
        ///
        /// # Transaction
        ///
        /// This method must be called within the context of a transaction.
        pub fn collect(&self) -> Vec<T>
        where
            T: Copy,
        {
            let all_items = self.all_items.lock();
            let current_tx = self.tx_provider.current();
            current_tx.data_with(|update: &DbUpdate<T>| {
                all_items.union(&update.new_items).cloned().collect()
            })
        }

        /// Returns the number of items in the DB.
        ///
        /// # Transaction
        ///
        /// This method must be called within the context of a transaction.
        pub fn len(&self) -> usize {
            let all_items = self.all_items.lock();
            let current_tx = self.tx_provider.current();
            let new_items_len = current_tx.data_with(|update: &DbUpdate<T>| update.new_items.len());
            all_items.len() + new_items_len
        }
    }

    #[test]
    fn commit_takes_effect() {
        let db: Db<u32> = Db::new();
        let items = vec![1, 2, 3];
        new_tx_and_insert_items::<u32, alloc::vec::IntoIter<u32>>(&db, items.clone().into_iter())
            .commit()
            .unwrap();
        assert!(collect_items(&db) == items);
    }

    #[test]
    fn abort_has_no_effect() {
        let db: Db<u32> = Db::new();
        let items = vec![1, 2, 3];
        new_tx_and_insert_items::<u32, alloc::vec::IntoIter<u32>>(&db, items.into_iter()).abort();
        assert!(collect_items(&db).len() == 0);
    }

    fn new_tx_and_insert_items<T, I>(db: &Db<T>, new_items: I) -> Tx
    where
        I: Iterator<Item = T>,
        T: Copy + Ord + 'static,
    {
        let mut tx = db.new_tx();
        tx.context(move || {
            for new_item in new_items {
                db.insert(new_item);
            }
        });
        tx
    }

    fn collect_items<T>(db: &Db<T>) -> Vec<T>
    where
        T: Copy + Ord + 'static,
    {
        let mut tx = db.new_tx();
        let items = tx.context(|| db.collect());
        tx.commit().unwrap();
        items
    }
}
