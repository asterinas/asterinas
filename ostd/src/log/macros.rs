// SPDX-License-Identifier: MPL-2.0

//! Logging macros.
//!
//! Contains the core [`log!`](crate::log!) macro and
//! level-specific wrappers (`info!`, `warn!`, etc.).
//! All are `#[macro_export]` so downstream crates can access them
//! via `use ostd::info;` etc.

/// Logs a message at the [`Emerg`] level.
///
/// [`Emerg`]: crate::log::Level::Emerg
#[macro_export]
macro_rules! emerg {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Emerg, $($arg)+) };
}

/// Logs a message at the [`Alert`] level.
///
/// [`Alert`]: crate::log::Level::Alert
#[macro_export]
macro_rules! alert {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Alert, $($arg)+) };
}

/// Logs a message at the [`Crit`] level.
///
/// [`Crit`]: crate::log::Level::Crit
#[macro_export]
macro_rules! crit {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Crit, $($arg)+) };
}

/// Logs a message at the [`Error`] level.
///
/// [`Error`]: crate::log::Level::Error
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Error, $($arg)+) };
}

/// Logs a message at the [`Warning`] level.
///
/// [`Warning`]: crate::log::Level::Warning
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Warning, $($arg)+) };
}

/// Logs a message at the [`Notice`] level.
///
/// [`Notice`]: crate::log::Level::Notice
#[macro_export]
macro_rules! notice {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Notice, $($arg)+) };
}

/// Logs a message at the [`Info`] level.
///
/// [`Info`]: crate::log::Level::Info
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Info, $($arg)+) };
}

/// Logs a message at the [`Debug`] level.
///
/// [`Debug`]: crate::log::Level::Debug
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Debug, $($arg)+) };
}

/// Returns `true` if a message at the given level would be logged.
///
/// Checks both the compile-time [`STATIC_MAX_LEVEL`] and the runtime
/// [`max_level`] filters.
///
/// [`STATIC_MAX_LEVEL`]: crate::log::STATIC_MAX_LEVEL
/// [`max_level`]: crate::log::max_level
#[macro_export]
macro_rules! log_enabled {
    ($level:expr) => {{
        let __level: $crate::log::Level = $level;
        $crate::log::STATIC_MAX_LEVEL.is_enabled(__level)
            && $crate::log::max_level().is_enabled(__level)
    }};
}

/// Logs a message at the given level.
///
/// This is the core logging macro.
/// All level-specific macros delegate to it.
///
/// The macro references a bare `__log_prefix!()` which resolves at
/// the call site via Rust's textual macro scoping.
/// The prefix is passed as a separate field on the [`Record`]
/// rather than concatenated into the format string,
/// so implicit format captures (`info!("{var}")`) work normally.
///
/// [`Record`]: crate::log::Record
///
/// # Examples
///
/// ```rust,ignore
/// use ostd::log::Level;
/// ostd::log!(Level::Warning, "unsupported");
/// ostd::log!(Level::Info, "value = {}", x);
/// ostd::log!(Level::Debug, "{name} started");
/// ```
#[macro_export]
macro_rules! log {
    ($level:expr, $($arg:tt)+) => {{
        // `const` is intentional: it enables compile-time dead code
        // elimination so that log calls above `STATIC_MAX_LEVEL` are
        // removed entirely.
        const __LEVEL: $crate::log::Level = $level;
        if $crate::log_enabled!(__LEVEL) {
            $crate::log::__write_log_record(&$crate::log::Record::new(
                __LEVEL,
                __log_prefix!(),
                format_args!($($arg)+),
                module_path!(),
                file!(),
                line!(),
            ));
        }
    }};
}

/// # How the `__log_prefix` mechanism works
///
/// The per-module prefix mechanism relies on Rust's textual
/// `macro_rules!` scoping rules. This document explains these rules,
/// the key constraints on `__log_prefix` definitions,
/// and why certain seemingly-reasonable alternatives don't work.
///
/// ## How the prefix reaches the output
///
/// The `log!` macro passes `__log_prefix!()` as a separate
/// `&'static str` field on [`Record`](crate::log::Record),
/// not as part of the `format_args!()` string.
/// The logger backend prepends it when formatting the message.
/// This avoids using `concat!()` inside `format_args!()`,
/// which would disable Rust's implicit format captures
/// (e.g., `info!("{var}")`).
///
/// ## Textual scoping of `macro_rules!`
///
/// A `macro_rules!` definition is visible to all items that appear
/// **after** it in the same file,
/// including file-backed child modules declared via `mod child;`:
///
/// ```rust,ignore
/// // lib.rs
/// macro_rules! __log_prefix { () => { "" }; }
/// mod sub;   // sub.rs can use __log_prefix!()
/// ```
///
/// A `macro_rules!` in an inner scope **shadows** one from an outer
/// scope. This enables per-module overrides:
///
/// ```rust,ignore
/// // lib.rs
/// macro_rules! __log_prefix { () => { "" }; }
/// mod sub;
///
/// // sub/mod.rs
/// macro_rules! __log_prefix { () => { "sub: " }; }
/// mod child;   // child.rs sees "sub: ", not ""
/// ```
///
/// ## How `__log_prefix!()` resolves through the macro chain
///
/// The `log!` macro contains a bare `__log_prefix!()` reference
/// (without `$crate::`). Bare names in `macro_rules!` expansions
/// resolve at the **call site**, not the definition site.
/// So when `info!("msg")` expands to
/// `$crate::log!(Level::Info, "msg")`,
/// the `__log_prefix!()` inside `log!` resolves at the site where
/// `info!()` was written — finding whichever `__log_prefix` is in
/// scope there (either the crate-root default or a module override).
///
/// ## Why `__log_prefix` must be at the crate root
///
/// The `__log_prefix` default must be defined at the crate root
/// (`lib.rs`), before any `mod` declarations, so that every module
/// in the crate inherits it via textual scoping.
/// Without it, any `info!()` call would fail with
/// "cannot find macro `__log_prefix`."
///
/// ## Why `__log_prefix` cannot be made more user-friendly
///
/// The raw `macro_rules! __log_prefix { ... }` boilerplate is admittedly
/// ugly. Two natural approaches to beautify it were explored and both
/// fail due to the same underlying Rust limitation.
///
/// **The underlying rule:** Rust's macro name resolution tracks whether
/// a `macro_rules!` item was directly written in source code or produced
/// by some form of macro expansion (including attribute processing).
/// Two definitions of the same name at different scopes are only allowed
/// to shadow each other if they are at the same "expansion level."
/// If one is directly written and the other is "macro-expanded,"
/// Rust reports E0659 (ambiguous).
///
/// ### Attempt 1: keep it one-line with `#[rustfmt::skip]`
///
/// Without `#[rustfmt::skip]`, `rustfmt` expands the definition to
/// multiple lines. It would be nice to write:
///
/// ```rust,ignore
/// #[rustfmt::skip]
/// macro_rules! __log_prefix { () => { "sub: " }; }
/// ```
///
/// But `#[rustfmt::skip]` (or any attribute) on a `macro_rules!` item
/// causes Rust to treat the item as "macro-expanded."
/// When the crate root has a directly-written default and a submodule
/// has an attributed override, the two are at different expansion levels
/// and Rust reports E0659:
///
/// ```rust,ignore
/// // lib.rs — directly written
/// macro_rules! __log_prefix { () => { "" }; }
///
/// // sub/mod.rs — attributed → "macro-expanded" → AMBIGUOUS
/// #[rustfmt::skip]
/// macro_rules! __log_prefix { () => { "sub: " }; }
/// ```
///
/// ### Attempt 2: hide behind a `set_log_prefix!` wrapper macro
///
/// A wrapper like `ostd::set_log_prefix!("iommu")` that expands to
/// `macro_rules! __log_prefix { ... }` would be much more ergonomic.
/// However, the expanded `macro_rules!` item is "macro-expanded"
/// (it was produced by expanding `set_log_prefix!`).
/// Two such definitions at different scopes — one from the crate root's
/// `set_log_prefix!` and one from a module's `set_log_prefix!` — are
/// always ambiguous (E0659), even though both originate from the same
/// wrapper macro.
///
/// ### Conclusion
///
/// The only way to get clean shadowing between scopes is for both the
/// default and the override to be **directly written** `macro_rules!`
/// items with no attributes and no wrapper macros.
mod about_log_prefix_macro {}
