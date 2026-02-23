# Rust Guidelines

Asterinas follows the
[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
and the project-specific conventions below.

- **[Naming](naming.md)** —
  CamelCase/acronym style
  and closure variable suffixes.
- **[Language Items](language-items/index.html)**
  - **[Variables, Expressions, and Statements](language-items/variables-expressions-and-statements.md)** —
    Explaining variables,
    block expressions,
    and checked arithmetic.
  - **[Functions and Methods](language-items/functions-and-methods.md)** —
    Nesting control,
    function focus,
    and boolean-argument avoidance.
  - **[Types and Traits](language-items/types-and-traits.md)** —
    Type-level invariants,
    enums for closed sets,
    and field encapsulation.
  - **[Comments and Documentation](language-items/comments-and-documentation.md)** —
    RFC 1574 summaries,
    comment/doc style,
    and module docs.
  - **[Unsafety](language-items/unsafety.md)** —
    `// SAFETY:` justification,
    `# Safety` docs,
    and module-boundary reasoning.
  - **[Modules and Crates](language-items/modules-and-crates.md)** —
    Visibility control
    and workspace dependencies.
  - **[Macros and Attributes](language-items/macros-and-attributes.md)** —
    Narrow lint suppression,
    `#[expect(dead_code)]` policy,
    and macro restraint.
- **[Select Topics](select-topics/index.html)**
  - **[Concurrency and Races](select-topics/concurrency-and-races.md)** —
    Lock ordering,
    spinlock discipline,
    atomics,
    and critical sections.
  - **[Defensive Programming](select-topics/defensive-programming.md)** —
    Correct use of `debug_assert!`.
  - **[Error Handling](select-topics/error-handling.md)** —
    Error propagation with `?`.
  - **[Logging](select-topics/logging.md)** —
    Standard logging macros and log-level selection.
  - **[Memory and Resource Management](select-topics/memory-and-resource-management.md)** —
    RAII and cleanup by ownership.
  - **[Performance](select-topics/performance.md)** —
    Hot-path complexity,
    copy/allocation control,
    and evidence-based optimization.
