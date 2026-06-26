# How Guidelines Are Written

The guidelines in this collection reflect
widely-recognized **philosophy**
for writing high-quality software.
Three books have influenced the guidelines the most:
1. [_The Art of Readable Code_](https://www.oreilly.com/library/view/the-art-of/9781449318482/)
2. [_Clean Code_](https://www.oreilly.com/library/view/clean-code-a/9780136083238/)
3. [_Code Complete_](https://stevemcconnell.com/books/)

The guidelines are derived from **code review experience**,
where we point out code smells, observe anti-patterns, and fix recurring bugs.
By the time the initial version of these guidelines was formulated,
Asterinas had seen thousands of code reviews.
We collected the historical review comments
as a [dataset](https://github.com/asterinas/pr-review-analysis)
and used it as both inspiration and evidence.

The remainder of this page has two parts.
The [Philosophy](#philosophy) section captures foundational beliefs
about what makes code understandable and maintainable.
The [Quality Criteria](#quality-criteria) section defines
how individual guidelines are written and accepted
into this collection.

## Philosophy {#philosophy}

### Minimize time to understand {#minimize-time-to-understand}

Code should be written to minimize the time
it would take someone else to fully understand it.
This is the fundamental theorem of readability
and the single most important measure
of code quality in this project.
"Someone else" includes your future self.

Code is read far more often than it is written.
If a technique makes code shorter
but harder to follow at a glance,
choose clarity over brevity.

### Managing complexity is the primary technical imperative {#managing-complexity}

No one can hold an entire modern program in their head.
The purpose of every technique in software construction —
decomposition, naming, encapsulation, abstraction —
is to break complex problems into simple pieces
so that you can safely focus on one thing at a time.

### Craftsmanship and care {#craftsmanship}

Clean code looks like it was written by someone who cares.
Professionalism means never knowingly leaving a mess.
The only way to go fast is to keep the code clean at all times.

### Continuous improvement {#continuous-improvement}

Leave code cleaner than you found it.
Small, steady improvements —
renaming a variable, extracting a function,
eliminating duplication —
prevent code from rotting over time.

## Quality Criteria {#quality-criteria}

Every guideline carries a **descriptive short name** in kebab-case
(e.g., `explain-variables`, `lock-ordering`).
Short names are kept **intact** even as the guidelines evolve
and should be used when referencing guidelines in code reviews.

A guideline is accepted into this collection
when it satisfies all four quality criteria:

1. **Concrete** —
   Framed as an actionable item with an illustrating example when possible.
2. **Concise** —
   Kept short; we do not want to intimidate readers.
3. **Grounded** —
   Opinionated or non-obvious guidelines should include a "See also" line
   with supportive materials (literature, PR reviews, codebase examples).
4. **Relevant** —
   Included only if it has codebase examples,
   prevents a past bug,
   or matches anti-patterns observed in code reviews.

When present, the **"See also" line** lists sources in the order:
literature; PR reviews; codebase examples.
Not every guideline needs all three;
for strongly opinionated or non-obvious guidelines,
include the line by default.

Do not add a guideline whose only value
is mechanical enforcement already provided by automated tools
such as [rustfmt](https://github.com/rust-lang/rustfmt) and [clippy](https://github.com/rust-lang/rust-clippy).
If a tool-enforced convention appears frequently in review
or needs project-specific rationale,
keep a short explanatory guideline and point to the tool configuration.

### Example

Below is a sample guideline demonstrating these conventions:

````md
### Introduce explaining variables (`explain-variables`) {#explain-variables}

Break down complex expressions
by assigning intermediate results to well-named variables.
An explaining variable turns an opaque expression
into self-documenting code:

```rust
// Good — intent is clear
let is_page_aligned = addr % PAGE_SIZE == 0;
let is_within_range = addr < max_addr;
debug_assert!(is_page_aligned && is_within_range);

// Bad — reader must parse the whole expression
debug_assert!(addr % PAGE_SIZE == 0 && addr < max_addr);
```

See also:
_The Art of Readable Code_, Chapter 8 "Breaking Down Giant Expressions";
PR [#2083](https://github.com/asterinas/asterinas/pull/2083#discussion_r2512772091)
and [#643](https://github.com/asterinas/asterinas/pull/643#discussion_r1497243812).
````

This example demonstrates:
- A stable short name (`explain-variables`) shown parenthetically and used as the heading anchor
- A one-sentence actionable recommendation followed by a code example
- A "See also" line ordered as: literature, PR reviews, codebase examples
