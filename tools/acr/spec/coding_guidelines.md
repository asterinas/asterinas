# Persona-Keyed Coding Guidelines

ACR does not invent its review standard.
It applies Asterinas's [Coding Guidelines](../../../book/src/to-contribute/coding-guidelines/),
an mdBook section that is the single source of truth.
ACR's [persona prompts](../src/prompts/personas/) describe *how* each persona works
but do not copy the rules.

## Why the guidelines are organized by persona

A rule such as "validate user input at the boundary" is both advice to the author
and an item the persona checks.
If each top-level guideline page is exactly one persona's responsibility,
the same page guides writing and serves as that persona's checklist.

A topic-based layout (Rust, Git, Testing, ...) cannot do this,
because topics and persona responsibilities cut across each other.
A "Rust" page mixes naming, `unsafe` soundness, and lock ordering,
which belong to three different personas;
meanwhile one persona's naming rules end up scattered over several topic pages.

Organizing by persona gives:

- **ownership**: every rule has one persona responsible for the failure it prevents;
- **stability**: persona roles change less often than languages, subsystems, or tools;
- **small contexts**: a review agent loads only its own persona's rules.

## The five personas

| Persona | Review question | Prompt |
|---|---|---|
| Maintainability | Is the code well-shaped and understandable over time? | [`maintainability.md`](../src/prompts/personas/maintainability.md) |
| Development | Is it correct and efficient on every input, error path, and interleaving, and is it proven by tests? | [`development.md`](../src/prompts/personas/development.md) |
| Security | Can hostile input or `unsafe` code break kernel security or soundness? | [`security.md`](../src/prompts/personas/security.md) |
| Hardware | Does low-level code obey the architecture, device, and ABI contracts? | [`hardware.md`](../src/prompts/personas/hardware.md) |
| Documentation | Are user-facing documents correct, up to date and well-written? | [`documentation.md`](../src/prompts/personas/documentation.md) |

The report titles Development section "Correctness".
Testing belongs to Development, because proving correctness is the developer's job.
Security and Hardware are split from general correctness
because they need adversarial reasoning and platform-specific knowledge.
Documentation covers user-facing artifacts (the book, the syscall-coverage files).

**Placement rule.**
A rule belongs to the persona that owns the failure it prevents, not to the persona its subject suggests.
For example, a module-documentation rule belongs to Maintainability when its purpose is to help future maintainers navigate the code.

Worked examples:

- `validate-at-boundaries` *reads* like a design rule,
  but the failure it prevents is *untrusted input crossing a trust boundary*
  — adversarial context the **Security** reviewer owns.
- `module-docs` *is about* documentation,
  but the failure it prevents is *a future maintainer cannot navigate this module*
  — and the maintainer holds the surrounding code context that judges that,
  so **Maintainability** owns it, not Documentation.

## Loading rules on demand

A persona has dozens of rules, and most are irrelevant to any given change.
ACR therefore loads them in two steps:

1. **Catalog.**
   Each review agent's instructions contain its persona's full catalog:
   every rule's short-name and one-line gist,
   plus a digest that identifies this exact version of the rules.
   The catalog is the agent's checklist.
2. **Exact text.**
   When the agent suspects a concrete violation,
   it calls the read-only `guideline.show` tool with the persona, the digest,
   and one or more candidate short-names,
   and receives the full text of those rules.

`guideline.show` fails if the digest is stale or a short-name is not in the catalog.
A finding may cite a short-name only after the agent has fetched it.
This keeps prompts small without losing traceability.
All backends use the same tool implementation
([`core/disclosure.py`](../src/core/disclosure.py) and [`scripts/print_guideline.py`](../src/scripts/print_guideline.py)).

Setting `ACR_GUIDELINE_DISCLOSURE=full` instead inlines every rule into the prompt.

## Which copy of the guidelines is used

The guidelines come from a trusted location chosen by [`run.sh`](../run.sh),
not necessarily from the checkout under review.

ACR uses the Coding Guidelines in the checkout that contains the run.sh you run.
Set ACR_GUIDELINE_ROOT to use another copy.

It fails if not contains the guidelines.
This lets you review an old commit with today's rules,
and lets the benchmark keep review rules out of the historical checkout
(see [Benchmark and evaluation](benchmark.md#keeping-answers-away-from-the-reviewer)).
