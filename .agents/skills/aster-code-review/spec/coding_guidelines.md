# The Persona-Keyed Coding Guidelines

`aster-code-review` does not invent its review standard;
it consumes Asterinas's **Coding Guidelines**,
an mdBook section under `book/src/to-contribute/coding-guidelines/`.
Those guidelines were deliberately reorganized
— from a topic-based layout to a **persona-based** one
— so that the *same* document that guides an author writing code also serves as a reviewer's checklist.
That reorganization is shipped (it predates this skill, on the `coding-guidelines-personas` branch);
this document explains its rationale,
because the skill's whole architecture rests on it.

## Coding and reviewing are two sides of one coin

A rule like "validate user input at the boundary" is simultaneously advice to the author and a thing the reviewer checks.
If the guidelines are structured so that each top-level page is exactly one reviewer's remit,
then the same page can guide writing *and* drive review
— for a human or an AI on either side.
That observation is the load-bearing idea:
organize the guidelines by **reviewer persona**,
and each persona's page doubles as that reviewer's checklist.

## Why the old topic-based layout could not be reviewed against

A code-review skill needs guidelines that are **comprehensive** (so few defects slip through uncovered)
and **digestible** (so a reviewer can load only the rules relevant to a given change, keeping the model's context window tight).
The original topic-based layout (General, Rust, Git, Testing, Assembly) was neither,
for one structural reason: a topic and a reviewer's remit **cross-cut** instead of nesting.

- **A topic is wider than a remit.**
  The "Rust" page bundled a naming concern (craft),
  an `unsafe`-soundness concern (security),
  a lock-ordering concern (correctness),
  and a logging concern (observability).
  No single reviewer owns "the Rust guidelines"
  — whoever is handed that page owns everything and therefore nothing.
- **A remit is wider than a topic.**
  Naming spilled across pages:
  `descriptive-names` sat under "General" while `camel-case-acronyms` sat under "Rust",
  though both are the *same* reviewer's job.

Because the partitions cross-cut,
every topic page was at once *too much* (rules beyond any one remit) and *too little* (rules of that remit stranded elsewhere).
So the layout was not **digestible**
— no page hands a reviewer just its remit
— and not reliably **comprehensive**
— a page no single reviewer owns leaves its rules everyone's job and therefore no one's,
and unowned rules are the ones that slip through.
The fix is to make the top-level partition *nest* under reviewer remits.

## The four criteria

The restructured guidelines must satisfy four criteria;
the design below is built to meet all four.

1. **Scale** — the structure must absorb hundreds of rules without becoming unnavigable.
2. **Stability** — the top-level categories must stay fixed as content grows;
   new rules and new repository paths must not force a top-level reshuffle.
3. **Locality** — language-specific and path-specific concerns must have a clear home (under the topic layout it was unclear where, e.g., a filesystem-specific rule should live).
4. **Digestibility** — a review agent must be able to consume it via two access patterns:
   **selective exposure** (load only the rules within one remit, never the whole rulebook)
   and **gradual exposure** (a short gist of each rule first, the full text only on demand).

## Three structural choices (and the criteria each satisfies)

**1.
The top level is reviewer personas.**
Top-level buckets are durable reviewer roles, not artifact kinds.
Roles are few and do not churn (**stability**),
and each role's page is a self-contained remit a reviewer can load alone (**selective exposure**).
Topics multiply with the codebase
— every new language or tool is a candidate topic
— and *review phases* ("audit resource lifecycles", "check locking") are an agent's volatile internal method;
**roles** are the one taxonomy that stays small and stable, historically five.

**2.
Each persona holds its own language- and path-specific rules**,
as deeper levels rather than new top-level siblings.
After a persona's language-agnostic subsections,
it may carry a **Rust-Specific** group (organized by language item — Naming, Types & Traits, …)
and a **Path-Specific** slot keyed on a repository path (an architecture directory, a subsystem, a sub-project like `ostd` or `kernel`).
This gives language/path concerns an unambiguous home (**locality**) and lets unbounded detail accrete *inside* a persona instead of widening the top level (**scale**).

**3.
Each persona page is a gist index.**
A persona's `README.md` is a bulleted list of `` [`short-name`](page#anchor):
<one-line gist> `` entries linking to the full rule pages.
A reader (or a review pass) grasps a rule from its gist and drills into the authored text only when a candidate violation warrants it (**gradual exposure**).

## The five personas

A kernel-development PR draws on a small, stable set of reviewer hats.
Each owns one top-level page and brings one remit to a review:

| Persona | Who they are | Remit |
|---|---|---|
| **Maintainability** | Must understand and safely change this code two years from now. | Is the shape of the change sound, and will the next reader understand it without archaeology? |
| **Development** | Makes the code correct under all inputs and schedules. | Does the code do the right thing — including on error, concurrent, and hot paths — and is it proven by tests? |
| **Security** | Assumes inputs are hostile and memory rules are exploitable. | Could an adversary breach the security of the kernel? |
| **Hardware** | Reviews assembly, ABI, and per-architecture invariants. | Is the low-level / arch-specific code correct against the hardware and ABI contract? |
| **Documentation** | Tends user-facing docs and compatibility artifacts. | Are user-facing docs and compatibility artifacts correct, current, and well-written? |

Two front-matter pages remain and are **not** personas:
the landing index (*how the guidelines are organized*)
and the philosophy / quality bar (*how guidelines are written* — a rule should be concrete, concise, grounded, relevant),
which governs every persona's rules and so sits above all five.

**Why exactly these five, and why they are stable.**
Maintainability and Development split the classic "is it well-shaped?" vs. "is it correct?" axis.
Security and Hardware are carved out of correctness because each needs **specialist context a generalist developer lacks**
— adversarial reasoning, and silicon/ABI knowledge.
Documentation covers user-facing artifacts (the book, the syscall-coverage files) that a purely defect-hunting review misses entirely.
These roles predate Asterinas and will not churn as it grows.

**Why no separate Tester persona.**
In Asterinas, writing and maintaining tests is part of a developer's job,
not a separate QA function: the engineer who makes the code correct also proves it with tests and keeps them green.
The Testing rules therefore live under **Development**
— a test failure is the same reviewer's failure as a logic bug,
judged with the same context.

## The placement principle

When a rule could plausibly sit in two personas, one test decides:

> **A rule belongs to the persona that is the natural owner of the failure it prevents**
> — the reviewer who would catch it,
> and whose bounded context holds the evidence to judge a violation.

Worked examples:

- `validate-at-boundaries` *reads* like a design rule,
  but the failure it prevents is *untrusted input crossing a trust boundary*
  — adversarial context the **Security** reviewer owns.
- `module-docs` *is about* documentation,
  but the failure it prevents is *a future maintainer cannot navigate this module*
  — and the maintainer holds the surrounding code context that judges that,
  so **Maintainability** owns it, not Documentation.

Same artifact medium, different *reader whose failure it is*.
The test is mechanical enough to settle every boundary without ad-hoc appeals
— which matters because a reviewer that does not know which page owns a concern is a reviewer that misses it.

## How this structure powers review

This is the payoff the skill cashes in (see [`execution_model.md`](execution_model.md)):

- **Selective exposure → one persona per pass.**
  Each persona maps to exactly one review pass that loads only that persona's page
  — a small, self-consistent context, never the whole rulebook.
  This is *why* the skill fans out into independent passes rather than running one prompt over all the rules.
- **Gradual exposure → gist-then-drill.**
  A pass reads each candidate rule's one-line gist first and queries the exact anchored rule section only on a suspected violation.
  The anchor it drills into is exactly the citation it puts in a guideline-backed comment (e.g. `for-development/concurrency.md#lock-ordering`),
  so a subjective call is automatically traceable to the standard.
- **Comprehensive coverage, including the non-defect concerns.**
  Because every rule has exactly one owning persona,
  no rule is unowned and therefore unchecked.
  And because the persona set spans more than defect-hunting
  — it includes Maintainability and Documentation
  — the review also catches readability and doc-currency problems that a purely bug-focused reviewer would miss.

The exhaustive per-persona rule index lives in the book itself (`book/src/to-contribute/coding-guidelines/`),
which is the single source of truth;
the skill's persona templates link to those pages rather than copying them.
