// SPDX-License-Identifier: MPL-2.0

//! A helper module for extremely simple synchronization primitives.

/// Define a spinlocked static mutable variable.
/// Example:
/// ```ignore
/// spinlock! {
///     pub static GLOBAL_COUNT: usize = 0;
/// }
/// ```
/// To access the variable, use [`lock_and`].
macro_rules! spinlock {
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr;) => (
        paste::paste! {
            $(#[$attr])*
            #[allow(non_upper_case_globals)]
            $vis static mut [<__ $name _val__>]: $t = $init;
            #[allow(non_upper_case_globals)]
            $vis static [<__ $name _lock__>]: core::sync::atomic::AtomicBool =
                core::sync::atomic::AtomicBool::new(false);
            #[allow(non_camel_case_types)]
            $vis type [<__ $name _type__>] = $t;
        }
    );
}
pub(crate) use spinlock;

/// Access a spinlocked static mutable variable using a closure.
/// Example:
/// ```ignore
/// spinlock! {
///     pub static GLOBAL_COUNT: usize = 0;
/// }
/// fn foo() {
///     lock_and(GLOBAL_COUNT -> |g| {
///         g += 1;
///     });
///     let _cnt_now = lock_and(GLOBAL_COUNT -> |g| g);
/// }
/// ```
macro_rules! lock_and {
    ($name:ident -> |$var:ident| $closure_body:expr) => {{
        use core::sync::atomic::Ordering;

        use paste::paste;

        // Acquire the lock.
        while paste! { [<__ $name _lock__>] }
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        let closure = |$var: &mut paste! { [<__ $name _type__>] }| $closure_body;
        // SAFETY: we have a locking mechanism to ensure exclusive access.
        let ret = closure(paste! { unsafe { &mut [<__ $name _val__>] } });
        // Release the lock.
        paste! { [<__ $name _lock__>] }.store(false, Ordering::Release);

        ret
    }};
}
pub(crate) use lock_and;
