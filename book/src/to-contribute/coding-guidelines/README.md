# Coding Guidelines

This section describes coding and collaboration conventions
for the Asterinas project.
These guidelines aim to keep code
clear, consistent, maintainable, correct, and efficient.

For the underlying philosophy, principles,
and quality criteria for individual guidelines, see
**[How Guidelines Are Written](how-guidelines-are-written.md)**.

The guidelines are organized into the following pages:

- **[General Guidelines](general-guidelines/index.html)** —
  Language-agnostic guidance for naming, comments,
  layout, formatting, and API design.
- **[Rust Guidelines](rust-guidelines/index.html)** —
  Rust-specific guidance for naming, language items,
  and cross-cutting topics.
- **[Git Guidelines](git-guidelines.md)** —
  Commit hygiene and pull-request conventions.
- **[Testing Guidelines](testing-guidelines.md)** —
  Test behavior, assertions, regression policy,
  and cleanup.
- **[Assembly Guidelines](asm-guidelines.md)** —
  `.S` and `global_asm!` conventions for sectioning,
  function metadata, labels, and alignment.

The guidelines represent the _desired_ state of the codebase.
If you find conflicting cases in the codebase,
you are welcome to fix the code to match the guidelines.

## Index

| Category | Guideline | Short Name |
|----------|-----------|------------|
| General | Be descriptive | [`descriptive-names`](general-guidelines/index.html#descriptive-names) |
| General | Be accurate | [`accurate-names`](general-guidelines/index.html#accurate-names) |
| General | Encode units and important attributes in names | [`encode-units`](general-guidelines/index.html#encode-units) |
| General | Use assertion-style boolean names | [`bool-names`](general-guidelines/index.html#bool-names) |
| General | Prefer semantic line breaks | [`semantic-line-breaks`](general-guidelines/index.html#semantic-line-breaks) |
| General | Explain why, not what | [`explain-why`](general-guidelines/index.html#explain-why) |
| General | Document design decisions | [`design-decisions`](general-guidelines/index.html#design-decisions) |
| General | Cite specifications and algorithm sources | [`cite-sources`](general-guidelines/index.html#cite-sources) |
| General | One concept per file | [`one-concept-per-file`](general-guidelines/index.html#one-concept-per-file) |
| General | Organize code for top-down reading | [`top-down-reading`](general-guidelines/index.html#top-down-reading) |
| General | Group statements into logical paragraphs | [`logical-paragraphs`](general-guidelines/index.html#logical-paragraphs) |
| General | Format error messages consistently | [`error-message-format`](general-guidelines/index.html#error-message-format) |
| General | Stick to familiar conventions | [`familiar-conventions`](general-guidelines/index.html#familiar-conventions) |
| General | Hide implementation details | [`hide-impl-details`](general-guidelines/index.html#hide-impl-details) |
| General | Validate at boundaries, trust internally | [`validate-at-boundaries`](general-guidelines/index.html#validate-at-boundaries) |
| Rust | Follow Rust CamelCase and acronym capitalization | [`camel-case-acronyms`](rust-guidelines/naming.md#camel-case-acronyms) |
| Rust | End closure variables with `_fn` | [`closure-fn-suffix`](rust-guidelines/naming.md#closure-fn-suffix) |
| Rust | Introduce explaining variables | [`explain-variables`](rust-guidelines/language-items/variables-expressions-and-statements.md#explain-variables) |
| Rust | Use block expressions to scope temporary state | [`block-expressions`](rust-guidelines/language-items/variables-expressions-and-statements.md#block-expressions) |
| Rust | Use checked or saturating arithmetic | [`checked-arithmetic`](rust-guidelines/language-items/variables-expressions-and-statements.md#checked-arithmetic) |
| Rust | Minimize nesting | [`minimize-nesting`](rust-guidelines/language-items/functions-and-methods.md#minimize-nesting) |
| Rust | Keep functions small and focused | [`small-functions`](rust-guidelines/language-items/functions-and-methods.md#small-functions) |
| Rust | Avoid boolean arguments | [`no-bool-args`](rust-guidelines/language-items/functions-and-methods.md#no-bool-args) |
| Rust | Use types to enforce invariants | [`rust-type-invariants`](rust-guidelines/language-items/types-and-traits.md#rust-type-invariants) |
| Rust | Prefer enum over trait objects for closed sets | [`enum-over-dyn`](rust-guidelines/language-items/types-and-traits.md#enum-over-dyn) |
| Rust | Encapsulate fields behind getters | [`getter-encapsulation`](rust-guidelines/language-items/types-and-traits.md#getter-encapsulation) |
| Rust | Follow RFC 1574 summary line conventions | [`rfc1574-summary`](rust-guidelines/language-items/comments-and-documentation.md#rfc1574-summary) |
| Rust | End sentence comments with punctuation | [`comment-punctuation`](rust-guidelines/language-items/comments-and-documentation.md#comment-punctuation) |
| Rust | Wrap identifiers in backticks | [`backtick-identifiers`](rust-guidelines/language-items/comments-and-documentation.md#backtick-identifiers) |
| Rust | Do not disclose implementation details in doc comments | [`no-impl-in-docs`](rust-guidelines/language-items/comments-and-documentation.md#no-impl-in-docs) |
| Rust | Add module-level documentation for major components | [`module-docs`](rust-guidelines/language-items/comments-and-documentation.md#module-docs) |
| Rust | Justify every use of `unsafe` | [`justify-unsafe-use`](rust-guidelines/language-items/unsafety.md#justify-unsafe-use) |
| Rust | Document safety conditions | [`document-safety-conds`](rust-guidelines/language-items/unsafety.md#document-safety-conds) |
| Rust | Deny unsafe code in `kernel/` | [`deny-unsafe-kernel`](rust-guidelines/language-items/unsafety.md#deny-unsafe-kernel) |
| Rust | Reason about safety at the module boundary | [`module-boundary-safety`](rust-guidelines/language-items/unsafety.md#module-boundary-safety) |
| Rust | Default to the narrowest visibility | [`narrow-visibility`](rust-guidelines/language-items/modules-and-crates.md#narrow-visibility) |
| Rust | Use workspace dependencies | [`workspace-deps`](rust-guidelines/language-items/modules-and-crates.md#workspace-deps) |
| Rust | Suppress lints at the narrowest scope | [`narrow-lint-suppression`](rust-guidelines/language-items/macros-and-attributes.md#narrow-lint-suppression) |
| Rust | Use `#[expect(dead_code)]` with restraint | [`expect-dead-code`](rust-guidelines/language-items/macros-and-attributes.md#expect-dead-code) |
| Rust | Prefer functions over macros | [`macros-as-last-resort`](rust-guidelines/language-items/macros-and-attributes.md#macros-as-last-resort) |
| Rust | Establish and enforce a consistent lock order | [`lock-ordering`](rust-guidelines/select-topics/concurrency-and-races.md#lock-ordering) |
| Rust | Never do I/O or blocking operations while holding a spinlock | [`no-io-under-spinlock`](rust-guidelines/select-topics/concurrency-and-races.md#no-io-under-spinlock) |
| Rust | Do not use atomics casually | [`careful-atomics`](rust-guidelines/select-topics/concurrency-and-races.md#careful-atomics) |
| Rust | Critical sections must not be split across lock boundaries | [`atomic-critical-sections`](rust-guidelines/select-topics/concurrency-and-races.md#atomic-critical-sections) |
| Rust | Use `debug_assert` for correctness-only checks | [`debug-assert`](rust-guidelines/select-topics/defensive-programming.md#debug-assert) |
| Rust | Propagate errors with `?` | [`propagate-errors`](rust-guidelines/select-topics/error-handling.md#propagate-errors) |
| Rust | Use `log` crate macros exclusively | [`log-crate-only`](rust-guidelines/select-topics/logging.md#log-crate-only) |
| Rust | Choose appropriate log levels | [`log-levels`](rust-guidelines/select-topics/logging.md#log-levels) |
| Rust | Use RAII for all resource acquisition and release | [`raii`](rust-guidelines/select-topics/memory-and-resource-management.md#raii) |
| Rust | Avoid O(n) algorithms on hot paths | [`no-linear-hot-paths`](rust-guidelines/select-topics/performance.md#no-linear-hot-paths) |
| Rust | Minimize unnecessary copies and allocations | [`minimize-copies`](rust-guidelines/select-topics/performance.md#minimize-copies) |
| Rust | No premature optimization without evidence | [`no-premature-optimization`](rust-guidelines/select-topics/performance.md#no-premature-optimization) |
| Git | Write imperative, descriptive subject lines | [`imperative-subject`](git-guidelines.md#imperative-subject) |
| Git | One logical change per commit | [`atomic-commits`](git-guidelines.md#atomic-commits) |
| Git | Separate refactoring from features | [`refactor-then-feature`](git-guidelines.md#refactor-then-feature) |
| Git | Keep pull requests focused | [`focused-prs`](git-guidelines.md#focused-prs) |
| Testing | Add regression tests for every bug fix | [`add-regression-tests`](testing-guidelines.md#add-regression-tests) |
| Testing | Test user-visible behavior, not internals | [`test-visible-behavior`](testing-guidelines.md#test-visible-behavior) |
| Testing | Use assertion macros, not manual inspection | [`use-assertions`](testing-guidelines.md#use-assertions) |
| Testing | Clean up resources after every test | [`test-cleanup`](testing-guidelines.md#test-cleanup) |
| Assembly | Use the correct section directive | [`asm-section-directives`](asm-guidelines.md#asm-section-directives) |
| Assembly | Place code-width directives after the section definition | [`asm-code-width`](asm-guidelines.md#asm-code-width) |
| Assembly | Place attributes directly before the function | [`asm-function-attributes`](asm-guidelines.md#asm-function-attributes) |
| Assembly | Add `.type` and `.size` for Rust-callable functions | [`asm-type-and-size`](asm-guidelines.md#asm-type-and-size) |
| Assembly | Use unique label prefixes to avoid name clashes | [`asm-label-prefixes`](asm-guidelines.md#asm-label-prefixes) |
| Assembly | Prefer `.balign` over `.align` | [`asm-prefer-balign`](asm-guidelines.md#asm-prefer-balign) |
