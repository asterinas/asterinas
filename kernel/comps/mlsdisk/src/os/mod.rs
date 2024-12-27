// SPDX-License-Identifier: MPL-2.0

//! OS-specific or OS-dependent APIs.

pub use alloc::{
    boxed::Box,
    collections::BTreeMap,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use aes_gcm::{
    aead::{AeadInPlace, Key, NewAead, Nonce, Tag},
    aes::Aes128,
    Aes128Gcm,
};
use ctr::cipher::{NewCipher, StreamCipher};
pub use hashbrown::{HashMap, HashSet};
pub use ostd::sync::{Mutex, MutexGuard, RwLock, SpinLock};
use ostd::{
    arch::read_random,
    sync::{self, PreemptDisabled, WaitQueue},
    task::{Task, TaskOptions},
};
use ostd_pod::Pod;
use serde::{Deserialize, Serialize};

use crate::{
    error::{Errno, Error},
    prelude::Result,
};

pub type RwLockReadGuard<'a, T> = sync::RwLockReadGuard<'a, T, PreemptDisabled>;
pub type RwLockWriteGuard<'a, T> = sync::RwLockWriteGuard<'a, T, PreemptDisabled>;
pub type SpinLockGuard<'a, T> = sync::SpinLockGuard<'a, T, PreemptDisabled>;
pub type Tid = u32;

/// A struct to get a unique identifier for the current thread.
pub struct CurrentThread;

impl CurrentThread {
    /// Returns the Tid of current kernel thread.
    pub fn id() -> Tid {
        let Some(task) = Task::current() else {
            return 0;
        };

        task.data() as *const _ as u32
    }
}

/// A `Condvar` (Condition Variable) is a synchronization primitive that can block threads
/// until a certain condition becomes true.
///
/// This is a copy from `aster-nix`.
pub struct Condvar {
    waitqueue: Arc<WaitQueue>,
    counter: SpinLock<Inner>,
}

struct Inner {
    waiter_count: u64,
    notify_count: u64,
}

impl Condvar {
    /// Creates a new condition variable.
    pub fn new() -> Self {
        Condvar {
            waitqueue: Arc::new(WaitQueue::new()),
            counter: SpinLock::new(Inner {
                waiter_count: 0,
                notify_count: 0,
            }),
        }
    }

    /// Atomically releases the given `MutexGuard`,
    /// blocking the current thread until the condition variable
    /// is notified, after which the mutex will be reacquired.
    ///
    /// Returns a new `MutexGuard` if the operation is successful,
    /// or returns the provided guard
    /// within a `LockErr` if the waiting operation fails.
    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> Result<MutexGuard<'a, T>> {
        let cond = || {
            // Check if the notify counter is greater than 0.
            let mut counter = self.counter.lock();
            if counter.notify_count > 0 {
                // Decrement the notify counter.
                counter.notify_count -= 1;
                Some(())
            } else {
                None
            }
        };
        {
            let mut counter = self.counter.lock();
            counter.waiter_count += 1;
        }
        let lock = MutexGuard::get_lock(&guard);
        drop(guard);
        self.waitqueue.wait_until(cond);
        Ok(lock.lock())
    }

    /// Wakes up one blocked thread waiting on this condition variable.
    ///
    /// If there is a waiting thread, it will be unblocked
    /// and allowed to reacquire the associated mutex.
    /// If no threads are waiting, this function is a no-op.
    pub fn notify_one(&self) {
        let mut counter = self.counter.lock();
        if counter.waiter_count == 0 {
            return;
        }
        counter.notify_count += 1;
        self.waitqueue.wake_one();
        counter.waiter_count -= 1;
    }

    /// Wakes up all blocked threads waiting on this condition variable.
    ///
    /// This method will unblock all waiting threads
    /// and they will be allowed to reacquire the associated mutex.
    /// If no threads are waiting, this function is a no-op.
    pub fn notify_all(&self) {
        let mut counter = self.counter.lock();
        if counter.waiter_count == 0 {
            return;
        }
        counter.notify_count = counter.waiter_count;
        self.waitqueue.wake_all();
        counter.waiter_count = 0;
    }
}

impl fmt::Debug for Condvar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Condvar").finish_non_exhaustive()
    }
}

/// Wrap the `Mutex` provided by kernel, used for `Condvar`.
#[repr(transparent)]
pub struct CvarMutex<T> {
    inner: Mutex<T>,
}

// TODO: add distinguish guard type for `CvarMutex` if needed.

impl<T> CvarMutex<T> {
    /// Constructs a new `Mutex` lock, using the kernel's `struct mutex`.
    pub fn new(t: T) -> Self {
        Self {
            inner: Mutex::new(t),
        }
    }

    /// Acquires the lock and gives the caller access to the data protected by it.
    pub fn lock(&self) -> Result<MutexGuard<'_, T>> {
        let guard = self.inner.lock();
        Ok(guard)
    }
}

impl<T: fmt::Debug> fmt::Debug for CvarMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("No data, since `CvarMutex` does't support `try_lock` now")
    }
}

/// Spawns a new thread, returning a `JoinHandle` for it.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + Sync + 'static,
    T: Send + 'static,
{
    let is_finished = Arc::new(AtomicBool::new(false));
    let data = Arc::new(SpinLock::new(None));

    let is_finished_clone = is_finished.clone();
    let data_clone = data.clone();
    let task = TaskOptions::new(move || {
        let data = f();
        *data_clone.lock() = Some(data);
        is_finished_clone.store(true, Ordering::Release);
    })
    .spawn()
    .unwrap();

    JoinHandle {
        task,
        is_finished,
        data,
    }
}

/// An owned permission to join on a thread (block on its termination).
///
/// This struct is created by the `spawn` function.
pub struct JoinHandle<T> {
    task: Arc<Task>,
    is_finished: Arc<AtomicBool>,
    data: Arc<SpinLock<Option<T>>>,
}

impl<T> JoinHandle<T> {
    /// Checks if the associated thread has finished running its main function.
    pub fn is_finished(&self) -> bool {
        self.is_finished.load(Ordering::Acquire)
    }

    /// Waits for the associated thread to finish.
    pub fn join(self) -> Result<T> {
        while !self.is_finished() {
            Task::yield_now();
        }

        let data = self.data.lock().take().unwrap();
        Ok(data)
    }
}

impl<T> fmt::Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinHandle").finish_non_exhaustive()
    }
}

/// A random number generator.
pub struct Rng;

impl crate::util::Rng for Rng {
    fn new(_seed: &[u8]) -> Self {
        Self
    }

    fn fill_bytes(&self, dest: &mut [u8]) -> Result<()> {
        let (chunks, remain) = dest.as_chunks_mut::<8>();
        chunks.iter_mut().for_each(|chunk| {
            chunk.copy_from_slice(read_random().unwrap_or(0u64).as_bytes());
        });
        remain.copy_from_slice(&read_random().unwrap_or(0u64).as_bytes()[..remain.len()]);
        Ok(())
    }
}

/// A macro to define byte_array_types used by `Aead` or `Skcipher`.
macro_rules! new_byte_array_type {
    ($name:ident, $n:expr) => {
        #[repr(C)]
        #[derive(Copy, Clone, Pod, Debug, Default, Deserialize, Serialize)]
        pub struct $name([u8; $n]);

        impl core::ops::Deref for $name {
            type Target = [u8];

            fn deref(&self) -> &Self::Target {
                self.0.as_slice()
            }
        }

        impl core::ops::DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                self.0.as_mut_slice()
            }
        }

        impl crate::util::RandomInit for $name {
            fn random() -> Self {
                use crate::util::Rng;

                let mut result = Self::default();
                let rng = self::Rng::new(&[]);
                rng.fill_bytes(&mut result).unwrap_or_default();
                result
            }
        }
    };
}

const AES_GCM_KEY_SIZE: usize = 16;
const AES_GCM_IV_SIZE: usize = 12;
const AES_GCM_MAC_SIZE: usize = 16;

new_byte_array_type!(AeadKey, AES_GCM_KEY_SIZE);
new_byte_array_type!(AeadIv, AES_GCM_IV_SIZE);
new_byte_array_type!(AeadMac, AES_GCM_MAC_SIZE);

/// An `AEAD` cipher.
#[derive(Debug, Default)]
pub struct Aead;

impl Aead {
    /// Construct an `Aead` instance.
    pub fn new() -> Self {
        Self
    }
}

impl crate::util::Aead for Aead {
    type Key = AeadKey;
    type Iv = AeadIv;
    type Mac = AeadMac;

    fn encrypt(
        &self,
        input: &[u8],
        key: &AeadKey,
        iv: &AeadIv,
        aad: &[u8],
        output: &mut [u8],
    ) -> Result<AeadMac> {
        let key = Key::<Aes128Gcm>::from_slice(key);
        let nonce = Nonce::<Aes128Gcm>::from_slice(iv);
        let cipher = Aes128Gcm::new(key);

        output.copy_from_slice(input);
        let tag = cipher
            .encrypt_in_place_detached(nonce, aad, output)
            .map_err(|_| Error::with_msg(Errno::EncryptFailed, "aes-128-gcm encryption failed"))?;

        let mut aead_mac = AeadMac::new_zeroed();
        aead_mac.copy_from_slice(&tag);
        Ok(aead_mac)
    }

    fn decrypt(
        &self,
        input: &[u8],
        key: &AeadKey,
        iv: &AeadIv,
        aad: &[u8],
        mac: &AeadMac,
        output: &mut [u8],
    ) -> Result<()> {
        let key = Key::<Aes128Gcm>::from_slice(key);
        let nonce = Nonce::<Aes128Gcm>::from_slice(iv);
        let tag = Tag::<Aes128Gcm>::from_slice(mac);
        let cipher = Aes128Gcm::new(key);

        output.copy_from_slice(input);
        cipher
            .decrypt_in_place_detached(nonce, aad, output, tag)
            .map_err(|_| Error::with_msg(Errno::DecryptFailed, "aes-128-gcm decryption failed"))
    }
}

type Aes128Ctr = ctr::Ctr128LE<Aes128>;

const AES_CTR_KEY_SIZE: usize = 16;
const AES_CTR_IV_SIZE: usize = 16;

new_byte_array_type!(SkcipherKey, AES_CTR_KEY_SIZE);
new_byte_array_type!(SkcipherIv, AES_CTR_IV_SIZE);

/// A symmetric key cipher.
#[derive(Debug, Default)]
pub struct Skcipher;

// TODO: impl `Skcipher` with linux kernel Crypto API.
impl Skcipher {
    /// Construct a `Skcipher` instance.
    pub fn new() -> Self {
        Self
    }
}

impl crate::util::Skcipher for Skcipher {
    type Key = SkcipherKey;
    type Iv = SkcipherIv;

    fn encrypt(
        &self,
        input: &[u8],
        key: &SkcipherKey,
        iv: &SkcipherIv,
        output: &mut [u8],
    ) -> Result<()> {
        let mut cipher = Aes128Ctr::new_from_slices(key, iv).unwrap();
        output.copy_from_slice(input);
        cipher.apply_keystream(output);
        Ok(())
    }

    fn decrypt(
        &self,
        input: &[u8],
        key: &SkcipherKey,
        iv: &SkcipherIv,
        output: &mut [u8],
    ) -> Result<()> {
        let mut cipher = Aes128Ctr::new_from_slices(key, iv).unwrap();
        output.copy_from_slice(input);
        cipher.apply_keystream(output);
        Ok(())
    }
}
