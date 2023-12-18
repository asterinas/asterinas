//! CPU.

use crate::trap::disable_local;
use core::cell::UnsafeCell;
use core::ops::Deref;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")]{
        pub use trapframe::GeneralRegs;
        pub use crate::arch::x86::cpu::*;
    }
}

/// Defines a CPU-local variable.
///
/// # Example
///
/// ```rust
/// use crate::cpu_local;
/// use core::cell::RefCell;
///
/// cpu_local! {
///     static FOO: RefCell<u32> = RefCell::new(1);
///
///     #[allow(unused)]
///     pub static BAR: RefCell<f32> = RefCell::new(1.0);
/// }
/// CpuLocal::borrow_with(&FOO, |val| {
///     println!("FOO VAL: {:?}", *val);
/// })
///
/// ```
#[macro_export]
macro_rules! cpu_local {
    // empty
    () => {};

    // multiple declarations
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => {
        $(#[$attr])* $vis static $name: CpuLocal<$t> = unsafe { CpuLocal::new($init) };
        $crate::cpu_local!($($rest)*);
    };

    // single declaration
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => (
        // TODO: reimplement cpu-local variable to support multi-core
        $(#[$attr])* $vis static $name: CpuLocal<$t> = CpuLocal::new($init);
    );
}

/// CPU-local objects.
///
/// A CPU-local object only gives you immutable references to the underlying value.
/// To mutate the value, one can use atomic values (e.g., `AtomicU32`) or internally mutable
/// objects (e.g., `RefCell`).
///
/// The `CpuLocal<T: Sync>` can be used directly.
/// Otherwise, the `CpuLocal<T>` must be used through `CpuLocal::borrow_with`.
///
/// TODO: re-implement `CpuLocal`
pub struct CpuLocal<T>(UnsafeCell<T>);

// Safety. At any given time, only one task can access the inner value T of a cpu-local variable.
unsafe impl<T> Sync for CpuLocal<T> {}

impl<T> CpuLocal<T> {
    /// Initialize CPU-local object
    /// Developer cannot construct a valid CpuLocal object arbitrarily
    #[allow(clippy::missing_safety_doc)]
    pub const unsafe fn new(val: T) -> Self {
        Self(UnsafeCell::new(val))
    }

    /// Borrow an immutable reference to the underlying value and feed it to a closure.
    ///
    /// During the execution of the closure, local IRQs are disabled. This ensures that
    /// the CPU-local object is only accessed by the current task or IRQ handler.
    /// As local IRQs are disabled, one should keep the closure as short as possible.
    pub fn borrow_with<U, F: FnOnce(&T) -> U>(this: &Self, f: F) -> U {
        // FIXME: implement disable preemption
        // Disable interrupts when accessing cpu-local variable
        let _guard = disable_local();
        // Safety. Now that the local IRQs are disabled, this CPU-local object can only be
        // accessed by the current task/thread. So it is safe to get its immutable reference
        // regardless of whether `T` implements `Sync` or not.
        let val_ref = unsafe { this.do_borrow() };
        f(val_ref)
    }

    unsafe fn do_borrow(&self) -> &T {
        &*self.0.get()
    }
}

impl<T: Sync> Deref for CpuLocal<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.0.get() }
    }
}
