# Motivation and Goals

## Why ACR exists

**Agents make code cheap; human review stays the bottleneck.**
Agent-assisted development sharply increases the amount of code reaching Asterinas,
but maintainers remain responsible for what is accepted.
Our policy is *"AI is welcome, but the human is responsible."*
That policy only holds if humans can actually keep up:
maintainers remain the gate of code acceptance,
and for quality assurance and long-term maintainability they must be that gate without drowning.
ACR takes over the broad, checklist-driven first pass
— the missing `// SAFETY:` comment, the unchecked user length —
so that maintainers can spend their attention on design and acceptance decisions.

**Agents that write kernel code need to review their own work.**
An agent iterating in a write → test → review loop improves only as far as its review step lets it.
ACR runs locally, without human interaction, so such a loop can call it after every round:
the loop commits each round and reviews its commits with `diff <base>`,
or reviews a specific uncommitted file with `files`.

These two users, a maintainer and an agent, share one artifact:
the Markdown report is readable by a human,
can be posted to a pull request,
and can be consumed by the loop as feedback
(see [Interface and output](interface.md)).

**Review is not only for changes.**
`files` mode audits existing code with no diff at all.
Periodically scanning risky modules catches latent defects that the original review missed.

## Goals

- **High recall.**
  Find as many real defects as possible:
  correctness bugs, security and soundness violations, hardware and ABI hazards,
  and guideline violations.
- **Useful precision.**
  Keep false alarms low enough that a report stays trustworthy.
  Verification removes only confidently refuted findings and marks the rest as uncertain.
- **Grounded, actionable findings.**
  Every finding names a concrete problem, assigns a severity, and proposes a fix.
  A guideline-based finding cites the rule's short-name.
- **Reproducible mechanics.**
  Everything except model judgement is deterministic and testable without a model.
- **Portable model execution.**
  Provider details stay behind the backend interface and the configuration file.
- **Inspectable runs.**
  Every run keeps its inputs, prompts, agent transcripts, token usage, and intermediate results,
  whether it succeeds or fails.
- **Evidence-based improvement.**
  The [benchmark](benchmark.md) measures recall on known defects,
  so changes to prompts and guidelines are judged by results.

## Non-goals

- **Not a CI gate or a web service.**
  This is the main contrast with Sashiko — see [related work](related_work.md).
- **Not a formatter, compiler, or linter.**
  `rustfmt`, Clippy, and similar tools already decide mechanical issues more reliably.
- **Not a replacement for the maintainer.**
  ACR reduces review effort; it does not accept a change.
- **Not a code executor.**
  ACR never builds or runs the code under review;
  agents only read source files and Git history.
