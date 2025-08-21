# Rust Guidelines

## API Documentation Guidelines

API documentation describes the meanings and usage of APIs,
and will be rendered into web pages by rustdoc.

It is necessary to add documentation to all public APIs,
including crates, modules, structs, traits, functions, macros, and more.
The use of the `#[warn(missing_docs)]` lint enforces this rule.

Asterinas adheres to the API style guidelines of the Rust community.
The recommended API documentation style can be found at
[How to write documentation - The rustdoc book](https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html).

## Lint Guidelines

Lints help us improve the code quality and find more bugs.
When suppressing lints, the suppression should affect as little scope as possible,
to make readers aware of the exact places where the lint is generated,
and to make it easier for subsequent committers to maintain such lint.

For example, if some methods in a trait are dead code,
marking the entire trait as dead code is unnecessary and
can easily be misinterpreted as the trait itself being dead code.
Instead, the following pattern is preferred:
```rust
trait SomeTrait {
    #[expect(dead_code)]
    fn foo();

    #[expect(dead_code)]
    fn bar();

    fn baz();
}
```

There is one exception:
If it is clear enough that every member will trigger the lint,
it is reasonable to expect the lint at the type level.
For example, in the following code,
we add `#[expect(non_camel_case_types)]` for the type `SomeEnum`,
instead of for each variant of the type:
```rust
#[expect(non_camel_case_types)]
enum SomeEnum {
    FOO_ABC,
    BAR_DEF,
}
```

### When to `#[expect(dead_code)]`

In general, dead code should be avoided because
_(i)_ it introduces unnecessary maintenance overhead, and
_(ii)_ its correctness can only be guaranteed by
manual and error-pruned review of the code.

In the case where expecting dead code is necessary,
it should fulfill the following requirements:
 1. We have a _concrete case_ that will be implemented in the future and
    will turn the dead code into used code.
 2. The semantics of the dead code are _clear_ enough
    (perhaps with the help of some comments),
    _even if the use case has not been added_.
 3. The dead code is _simple_ enough that
    both the committer and the reviewer can be confident that
    the code must be correct _without even testing it_.
 4. It serves as a counterpart to existing non-dead code.

For example, it is fine to add ABI constants that are unused because
the corresponding feature (_e.g.,_ a system call) is partially implemented.
This is a case where all of the above requirements are met,
so adding them as dead code is perfectly acceptable.
