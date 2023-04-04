use crate::sync::disable_local;
use core::cell::UnsafeCell;

/// Defines a CPU-local variable.
/// # Example
///
/// ```rust
/// use crate::cpu_local;
/// use core::cell::RefCell;
/// {
///     cpu_local! {
///         static FOO: RefCell<u32> = RefCell::new(1);
///
///         #[allow(unused)]
///         pub static BAR: RefCell<f32> = RefCell::new(1.0);
///     }
///
///     FOO.borrow_with(|val| {
///         println!("FOO VAL: {:?}", *val);
///     });
/// }
/// ```
#[macro_export]
macro_rules! cpu_local {
    // empty
    () => {};

    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const { $init:expr }; $($rest:tt)*) => {
        $(#[$attr])* $vis static $name: PerCpu<$t> = PerCpu::new(const $init);
        crate::cpu_local!($($rest)*);
    };

    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const { $init:expr }) => (
        $(#[$attr])* $vis static $name: PerCpu<$t> = PerCpu::new(const $init);
    );

    // multiple declarations
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => {
        $(#[$attr])* $vis static $name: PerCpu<$t> = PerCpu::new($init);
        crate::cpu_local!($($rest)*);
    };

    // single declaration
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => (
        // TODO: reimplement per-cpu variable to support multi-core
        $(#[$attr])* $vis static $name: PerCpu<$t> = PerCpu::new($init);
    );
}

/// Per-CPU objects.
///
/// A per-CPU object only gives you immutable references to the underlying value.
/// To mutate the value, one can use atomic values (e.g., `AtomicU32`) or internally mutable
/// objects (e.g., `RefCell`).
///
/// TODO: reimplement PerCpu inner UnsafeCell with access function as thread_local! macro,
/// because different CPUs access static per-cpu variables in different positions.
pub struct PerCpu<T>(UnsafeCell<T>);
// safety
unsafe impl<T> Sync for PerCpu<T> {}

impl<T> PerCpu<T> {
    /// Initialize per-CPU object
    /// unsafe
    pub const fn new(val: T) -> Self {
        Self(UnsafeCell::new(val))
    }
    /// Borrow an immutable reference to the underlying value and feed it to a closure.
    ///
    /// During the execution of the closure, local IRQs are disabled. This ensures that
    /// the per-CPU object is only accessed by the current task or IRQ handler.
    /// As local IRQs are disabled, one should keep the closure as short as possible.
    pub fn borrow_with<U, F: FnOnce(&T) -> U>(&self, f: F) -> U {
        // FIXME: implement disable preemption
        // Disable interrupts when accessing per-cpu variable
        let _guard = disable_local();
        // Safety. Now that the local IRQs are disabled, this per-CPU object can only be
        // accessed by the current task/thread. So it is safe to get its immutable reference
        // regardless of whether `T` implements `Sync` or not.
        let val_ref = unsafe { self.do_borrow() };
        f(val_ref)
    }

    unsafe fn do_borrow(&self) -> &T {
        &*self.0.get()
    }
}

impl<T: Sync> PerCpu<T> {
    /// Gets an immutable reference to the value inside the per-CPU object.
    pub fn borrow(&self) -> &T {
        // Safety. Since the value of `T` is `Sync`, it is ok for multiple tasks or IRQ handlers
        // executing on the current CPU to have immutable references.
        unsafe { self.do_borrow() }
    }
}
