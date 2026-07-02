# Motivation & Goals

## Why a code-review skill

**Agents make code cheap; human review is the bottleneck that stays.**
The AI boom has sharply increased the throughput of code reaching open-source projects,
Asterinas included.
Our policy is *"AI is welcome, but the human is responsible."*
That policy only holds if humans can actually keep up:
maintainers remain the gate of code acceptance,
and for quality assurance and long-term maintainability they must be that gate without drowning.
A good code-review skill exists to **carry the mechanical and checklist-driven part of that burden**
— the broad, tireless first pass
— so a human maintainer spends their scarce attention on judgement calls,
not on catching a missing `// SAFETY:` comment or an unchecked `min()` on a user buffer.

**Long-horizon kernel work needs agents that review their own code.**
The larger prize is autonomous,
long-horizon development: an agent running a write → test → review loop (a "Ralph loop") that keeps iterating on the code it writes until it converges.
Such a loop is only as good as its review step
— an agent that cannot critique its own diff cannot improve it.
A review skill the agent can invoke on its own changes,
repeatedly, is one component that closes that loop
— the write and test steps are the others,
so a full loop needs more than a review skill.
This is why the skill must run locally, fast, and be callable without human interaction.
The loop **commits each round** and reviews its commit series with `diff <base>` ([`interface.md`](interface.md))
— judging per-commit intent and commit hygiene,
not just the net change — or a specific uncommitted edit with `files`.
Making the surrounding loop fully first-class
— orchestrating the write and test steps,
keeping its scratch files out of review
— is the remaining work (see the [TODO](README.md#todo)).

These two audiences — the human maintainer triaging a pull request and the agent grinding a loop
— are the **same skill viewed from two sides**, and the design serves both.
The review file is the shared artifact:
a human reads it (or posts it as inline PR comments);
the loop reads it as its feedback signal (see [`interface.md`](interface.md)).

**And review is not only for changes.**
The same skill audits *existing* code on demand:
point it at a set of files and it reviews them against the guidelines, no diff required.
That makes it a standing quality tool
— periodically scan the riskiest modules to catch drift and latent defects the original review missed
— which is the second of the skill's two review modes (`files`; see [`interface.md`](interface.md)).

## Goals

- **Comprehensive recall.**
  Surface as many real defects as possible
  — correctness bugs, unsafe-soundness violations,
  ABI hazards, and guideline violations
  — across the whole of the code under review (a change or a set of files).
- **Precision, increasingly.**
  Keep false alarms low so a review stays trustworthy;
  as the benchmark gains false-positive problems,
  precision becomes a measured goal alongside recall.
- **Grounded, actionable output.**
  Every comment names *what* is wrong,
  cites the standard or bug it rests on,
  and ends with a concrete fix a human or the loop can apply directly.
- **Trustworthy by measurement.**
  The skill's effectiveness is not asserted;
  it is measured by a benchmark of known defects,
  with recall as the headline metric (see [`benchmark.md`](benchmark.md)).
- **Agent-agnostic and local-first.**
  One package, two agents (Claude Code and Codex), no server, no PR dependency.

## Non-goals

- **Not a CI gate or a web service.**
  It is a skill invoked in an agent session, not a hosted bot.
  (This is the main contrast with Sashiko — see [related work](related_work.md).)
- **Not a style formatter or linter.**
  Mechanical, tool-checkable issues (formatting, clippy lints) are left to existing tooling;
  the skill spends its attention on what those tools cannot judge.
- **Not a replacement for the human gate.**
  It reduces the manual review burden; it does not assume responsibility for acceptance.
  "AI is welcome, but the human is responsible" still holds.
