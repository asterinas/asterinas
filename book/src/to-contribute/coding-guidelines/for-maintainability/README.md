# For Maintainability

*Is the shape of the change sound, and will the next reader understand it without archaeology?*

This is the index of the **maintainability** guidelines.
Each subsection is its own page,
and each entry below links a stable `short-name` to its guideline,
with a one-line gist so a reader (or a review tool) can grasp the guideline before opening it.

## Index

**[Design](design.md)**
- [`single-responsibility`](design.md#single-responsibility): Each module, type, or function has one reason to change; keep functions small and one concept per file.
- [`dry`](design.md#dry): Give every piece of knowledge a single representation; eliminate duplication once a pattern recurs.
- [`information-hiding`](design.md#information-hiding): Hide details behind interfaces; expose only what consumers need.
- [`open-closed`](design.md#open-closed): Extend stable modules through existing interfaces; don't add extension points preemptively.
- [`least-surprise`](design.md#least-surprise): Names, types, and APIs behave as they suggest; prefer conventions users already know from Rust and Linux.
- [`coupling-cohesion`](design.md#coupling-cohesion): Keep inter-module connections small and visible; keep each module focused on one purpose.
- [`consistency`](design.md#consistency): Do similar things the same way; follow an existing convention rather than coining a competing one.
- [`rust-native`](design.md#rust-native): Learn from Linux's design, not its C idioms; write idiomatic Rust.

**[Process](process.md)**
- [`imperative-subject`](process.md#imperative-subject): Write each commit subject in the imperative mood, ≤72 chars, verb-first ("Fix", "Add", "Remove"); backtick identifiers.
- [`atomic-commits`](process.md#atomic-commits): One logical change per commit; don't mix unrelated changes.
- [`refactor-then-feature`](process.md#refactor-then-feature): Put preparatory refactoring in its own earlier commit(s), separate from the feature.
- [`focused-prs`](process.md#focused-prs): Keep a PR on a single topic; ensure CI passes before requesting review.

**[Naming](naming.md)**
- [`descriptive-names`](naming.md#descriptive-names): Names convey meaning at the point of use; avoid single letters and ambiguous abbreviations.
- [`accurate-names`](naming.md#accurate-names): Avoid names that mislead about meaning, behavior, or side effects.
- [`no-magic-number`](naming.md#no-magic-number): Give semantic names to numbers that encode rules, limits, units, masks, or external constants.
- [`encode-units`](naming.md#encode-units): When the type doesn't carry the unit, put it in the name (`timeout_ns`, `size_pages`).
- [`bool-names`](naming.md#bool-names): Name booleans as positive assertions (`is_`/`has_`/`can_`/…); avoid negation.
- [`error-message-format`](naming.md#error-message-format): Lowercase start (unless a proper noun), specific, Linux man-page style for syscall errors.

**[Layout](layout.md)**
- [`top-down-reading`](layout.md#top-down-reading): Order a file top-down: entry points and core flow first, detail below.
- [`logical-paragraphs`](layout.md#logical-paragraphs): Group related statements into blank-line-separated paragraphs, each one sub-step.

**[Comments](comments.md)**
- [`explain-why`](comments.md#explain-why): Comments explain intent (why), not what the code does; if you must explain "what", rewrite the code.
- [`design-decisions`](comments.md#design-decisions): Document non-obvious choices, with rationale and alternatives considered.
- [`cite-sources`](comments.md#cite-sources): Cite the source (POSIX, Linux man page, hardware manual, paper) for spec/algorithm behavior.

**[Rust-Specific](rust-specific/)**
- [Naming](rust-specific/naming.md)
    - [`camel-case-acronyms`](rust-specific/naming.md#camel-case-acronyms): Use Rust CamelCase with title-cased acronyms (`Nvme`, not `NVME`).
    - [`closure-fn-suffix`](rust-specific/naming.md#closure-fn-suffix): End a variable holding a closure or fn pointer with `_fn`.
- [Crates & Modules](rust-specific/crates-and-modules.md)
    - [`workspace-deps`](rust-specific/crates-and-modules.md#workspace-deps): Declare shared dependencies in `[workspace.dependencies]` and reference them with `.workspace = true`.
    - [`module-docs`](rust-specific/crates-and-modules.md#module-docs): Open a major module with a `//!` doc: purpose, key types, relation to neighbors.
    - [`narrow-visibility`](rust-specific/crates-and-modules.md#narrow-visibility): Start private; widen visibility only when an actual consumer requires it.
    - [`qualified-fn-imports`](rust-specific/crates-and-modules.md#qualified-fn-imports): Import the parent module and call free functions/statics through it, not by bare name.
- [Types & Traits](rust-specific/types-and-traits.md)
    - [`rust-type-invariants`](rust-specific/types-and-traits.md#rust-type-invariants): Use the type system (newtypes, enums, generics) to make illegal states unrepresentable.
    - [`enum-over-dyn`](rust-specific/types-and-traits.md#enum-over-dyn): For a closed set of variants, prefer an `enum` over `Box<dyn Trait>`.
    - [`getter-encapsulation`](rust-specific/types-and-traits.md#getter-encapsulation): Prefer a getter over a public field; it preserves naming freedom and room for invariants.
- [Functions & Methods](rust-specific/functions-and-methods.md)
    - [`no-bool-args`](rust-specific/functions-and-methods.md#no-bool-args): Avoid boolean parameters that select behavior; split the function or use a typed enum.
    - [`block-expressions`](rust-specific/functions-and-methods.md#block-expressions): Use a block expression to scope temporary state that only produces one value.
    - [`minimize-nesting`](rust-specific/functions-and-methods.md#minimize-nesting): Flatten nesting past ~3 levels with early returns, guard clauses, `let…else`, `?`, `continue`.
    - [`explain-variables`](rust-specific/functions-and-methods.md#explain-variables): Bind intermediate results of a complex expression to well-named variables.
- [Attributes & Macros](rust-specific/attributes-and-macros.md)
    - [`expect-dead-code`](rust-specific/attributes-and-macros.md#expect-dead-code): Allow `#[expect(dead_code)]` only for a planned, clear, simple future use.
    - [`alphabetical-attrs`](rust-specific/attributes-and-macros.md#alphabetical-attrs): Sort outer attributes alphabetically; place `#[derive(...)]` last with sorted traits.
    - [`narrow-lint-suppression`](rust-specific/attributes-and-macros.md#narrow-lint-suppression): Suppress a lint at the narrowest scope (item/method), not a whole type or module.
    - [`macros-as-last-resort`](rust-specific/attributes-and-macros.md#macros-as-last-resort): Prefer functions and generics; use a macro only when the type system can't express the need.
- [Comments](rust-specific/comments.md)
    - [`rfc1574-summary`](rust-specific/comments.md#rfc1574-summary): First doc line is one sentence — a third-person verb for functions, a noun phrase for types/modules.
    - [`comment-punctuation`](rust-specific/comments.md#comment-punctuation): End full-sentence comments with terminal punctuation.
    - [`backtick-identifiers`](rust-specific/comments.md#backtick-identifiers): Wrap identifiers in doc comments in backticks; prefer rustdoc links where possible.
    - [`no-impl-in-docs`](rust-specific/comments.md#no-impl-in-docs): Doc comments describe what an API does and how to use it, not its internal implementation.

No **path-specific** guidelines yet.
