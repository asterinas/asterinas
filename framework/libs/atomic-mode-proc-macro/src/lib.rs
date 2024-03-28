// SPDX-License-Identifier: MPL-2.0

//! This crate provides macros to help writing atomic-mode-safe code.

extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Expr, ItemFn};

/// The name of the default atomic-mode guard factory function
const DEFAULT_ATOMIC_GUARD_FACTORY_NAME: &str = "enter_atomic_mode";

/// Marks a function as being executed under the atomic mode.
/// The marked function enters the atomic mode context automatically
/// before the execution of its function body,
/// and exits the atomic mode when the function returns.
///
/// <div class="note">
///     Nested calls of functions with this attribute is permitted.
///     The CPU shall exit the atomic mode
///     only after the last atomic mode context ends.
/// </div>
///
/// # Argument
///
/// - `arg`: The function that creates an atomic-mode guard.
///             It should be a function or a path to a function.
///             If it is not provided, the default function `enter_atomic_mode`
///             will be used when there is one in the scope.
///             Avoid using functions with the same name as the default one.
///
/// # Examples
///
/// Use the default atomic mode guard factory function defined in the scope.
///
/// ```rust
/// struct AtomicModeGuard;
/// fn enter_atomic_mode() -> AtomicModeGuard {
///     AtomicModeGuard {}
/// }
///
/// #[atomic_mode_proc_macro::atomic_procedure]
/// fn atomic_function() {
///     // The function body is executed in atomic mode,
///     // i.e., guarded by the `AtomicModeGuard` until the function returns.
/// }
/// ```
///
/// Or, specify the guard factory function via the argument.
///
/// ```rust
/// struct AtomicModeGuard;
/// fn custom_enter_atomic_mode() -> AtomicModeGuard {
///     AtomicModeGuard {}
/// }
///
/// #[atomic_mode_proc_macro::atomic_procedure(custom_enter_atomic_mode)]
/// fn atomic_function() -> i8 {
///     1
/// }
/// ```
#[proc_macro_attribute]
pub fn atomic_procedure(arg: TokenStream, input: TokenStream) -> TokenStream {
    let factory = if arg.is_empty() {
        syn::parse_str::<Expr>(DEFAULT_ATOMIC_GUARD_FACTORY_NAME).unwrap()
    } else {
        parse_macro_input!(arg as Expr)
    };

    let atomic_gaurd = match factory {
        Expr::Path(_) | Expr::Closure(_) => quote! {
            let _atomic_mode_guard = #factory();
        },
        _ => quote! {
            compile_error!("Expects an expression representing a function.");
        },
    };

    exec_at_begin(atomic_gaurd, input)
}

/// The name of the default break-atomic-mode function
const DEFAULT_CHECK_NAME: &str = "might_break_atomic_mode";

/// Marks a function as a potential atomic-mode breaker.
/// A check will be performed before the execution of the function body,
/// and a panic will be triggered if the context where the function is called
/// is in the atomic mode.
///
/// # Argument
///
/// - `arg`: The function that panics if the current context is in the atomic mode.
///             It should be a function or a path to a function.
///             If it is not provided, the default function `might_break_atomic_mode`
///             will be used when there is one in the scope.
///             Avoid using functions with the same name as the default one.
///
/// # Examples
///
/// Use the default check function defined in the scope.
///
/// ```rust
/// fn might_break_atomic_mode() {
///     // check if the current context is in the atomic mode,
///     // and panic if it is
/// }
///
/// #[atomic_mode_proc_macro::might_break]
/// fn schedule() {
///     // The check happens at the beginning.
///     // Executes only if the current context is not in the atomic mode.
/// }
/// ```
///
/// Or, specify the check function via the argument.
///
/// ```rust
/// fn custom_might_break_atomic_mode() {
///     // check, panic if in atomic mode
/// }
///
/// #[atomic_mode_proc_macro::might_break(custom_might_break_atomic_mode)]
/// fn schedule() {
///     // `custom_might_break_atomic_mode`
///     // ....
/// }
/// ```
#[proc_macro_attribute]
pub fn might_break(arg: TokenStream, input: TokenStream) -> TokenStream {
    let destructive_check_expr = if arg.is_empty() {
        syn::parse_str::<Expr>(DEFAULT_CHECK_NAME).unwrap()
    } else {
        parse_macro_input!(arg as Expr)
    };

    let destructive_check = match destructive_check_expr {
        Expr::Path(_) | Expr::Closure(_) => quote! {
            let _atomic_breakable_check: fn() = #destructive_check_expr;
            _atomic_breakable_check();
        },
        _ => quote! {
            compile_error!("Expects an expression representing a function.");
        },
    };

    exec_at_begin(destructive_check, input)
}

fn exec_at_begin(foreword: proc_macro2::TokenStream, func_token: TokenStream) -> TokenStream {
    let func = parse_macro_input!(func_token as ItemFn);
    let function_name = &func.sig.ident;
    let visibility = &func.vis;
    let generics = &func.sig.generics;
    let constness = &func.sig.constness;
    let inputs = &func.sig.inputs;
    let output = &func.sig.output;
    let body = &func.block;
    let attributes = &func.attrs;
    let constraints = &func.sig.generics.where_clause;
    quote! {
        #(#attributes)*
        #visibility #constness fn #function_name #generics (#inputs) #output
        #constraints
        {
            #foreword
            #body
        }
    }
    .into()
}
