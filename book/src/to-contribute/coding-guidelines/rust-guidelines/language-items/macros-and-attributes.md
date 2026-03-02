# Macros and Attributes

### Suppress lints at the narrowest scope (`narrow-lint-suppression`) {#narrow-lint-suppression}

When suppressing lints,
the suppression should affect as little scope as possible.
This makes readers aware
of the exact places where the lint is generated
and makes it easier for subsequent committers
to maintain the suppression.

```rust
// Good — each method is individually marked
trait SomeTrait {
    #[expect(dead_code)]
    fn foo();

    #[expect(dead_code)]
    fn bar();

    fn baz();
}

// Bad — the entire trait is suppressed
#[expect(dead_code)]
trait SomeTrait { ... }
```

There is one exception:
if it is clear enough
that every member will trigger the lint,
it is reasonable to expect the lint at the type level.

```rust
#[expect(non_camel_case_types)]
enum SomeEnum {
    FOO_ABC,
    BAR_DEF,
}
```

See also:
[Clippy `allow_attributes`](https://rust-lang.github.io/rust-clippy/master/#allow_attributes),
[Clippy `allow_attributes_without_reason`](https://rust-lang.github.io/rust-clippy/master/#allow_attributes_without_reason),
and rustc [`unfulfilled_lint_expectations`](https://doc.rust-lang.org/rustc/lints/listing/warn-by-default.html#unfulfilled-lint-expectations).

### Use `#[expect(dead_code)]` with restraint (`expect-dead-code`) {#expect-dead-code}

In general, dead code should be avoided because
_(i)_ it introduces unnecessary maintenance overhead, and
_(ii)_ its correctness can only be guaranteed
by manual and error-prone review.

Dead code is acceptable only when all of these hold:

1. A _concrete case_ will be implemented in the future
   that turns the dead code into used code.
2. The semantics are _clear_ enough,
   even without the use case.
3. The dead code is _simple_ enough
   that both the committer and the reviewer
   can be confident it is correct without testing.
4. It serves as a counterpart to existing non-dead code.

For example, it is fine to add ABI constants
that are unused because the corresponding feature
is partially implemented.

See also:
[Rust Reference: Diagnostic attributes](https://doc.rust-lang.org/reference/attributes/diagnostics.html)
and rustc [`unfulfilled_lint_expectations`](https://doc.rust-lang.org/rustc/lints/listing/warn-by-default.html#unfulfilled-lint-expectations).

### Prefer functions over macros (`macros-as-last-resort`) {#macros-as-last-resort}

Prefer functions and generics over macros.
Macros are powerful
but harder to understand, debug, test, and format.
Reach for a macro only when
the type system or generics cannot express
what you need
(e.g., variadic arguments, compile-time code generation,
or DSL syntax).

```rust
// Good — a generic function covers all types
fn align_up<T: Into<usize>>(val: T, align: usize) -> usize {
    let val = val.into();
    (val + align - 1) & !(align - 1)
}

// Bad — a macro where a function would suffice
macro_rules! align_up {
    ($val:expr, $align:expr) => {
        ($val + $align - 1) & !($align - 1)
    };
}
```

See also:
_The Rust Programming Language_, Chapter 20.5 "Macros";
[Rust by Example: Macros](https://doc.rust-lang.org/rust-by-example/macros.html).
