# `aster-code-review` — Design Specification

This is the authoritative design of `aster-code-review`,
a code-review skill for Asterinas.
It is written to be **self-contained** (a reader needs nothing but this `spec/` to understand the design) and **justified** (every significant decision is argued in place, including the alternatives that were rejected and why).

## What the skill is, in one breath

`aster-code-review` reviews code against Asterinas's **persona-keyed Coding Guidelines** and writes one Markdown **review file**.
It works in two modes — a Git change (`diff`) or a set of target files (`files`)
— both reviewed relative to the current checkout (**HEAD**):
`diff` reviews the commits up to HEAD,
`files` the working-tree files at HEAD.
Five reviewer **personas** — Maintainability,
Development, Security, Hardware,
Documentation — each own one slice of the guidelines and review the code independently;
a thin orchestrator fans them out,
then verifies, consolidates,
and summarizes their findings into the review file.

Four properties shape everything else:

- **Local-first** — no web server,
  no PR; it reads the repo and writes a file,
  so the same skill serves a human at a terminal and an agent in a loop.
- **Agent-agnostic** — one package runs under both Claude Code and Codex;
  only the primitive that spawns a sub-task differs.
- **Persona-based** — the change is reviewed by independent reviewer personas,
  each carrying one bounded, self-contained slice of the guidelines.
- **Benchmark-driven** — quality is measured,
  not asserted: a benchmark of known defects scores the skill's recall (catching as many real defects as possible) today,
  and will grow to also score precision (raising few false alarms).

## How to read this spec

| Document | Covers |
|---|---|
| [`motivation.md`](motivation.md) | Why this skill exists, the two audiences it serves, and its goals and non-goals. |
| [`coding-guidelines.md`](coding-guidelines.md) | The persona-keyed Coding Guidelines the skill consumes — why they are organized by persona, the five personas, and how that structure makes review tractable. |
| [`interface.md`](interface.md) | How the skill is invoked, its two review modes (`diff` and `files`) and the *HEAD is the head* rule, and the format and comment model of the review file it produces. |
| [`execution-model.md`](execution-model.md) | The orchestration pipeline: persona fan-out, deterministic assembly, the verification and consolidation passes, persona activation, and agent-agnostic packaging. |
| [`benchmark.md`](benchmark.md) | How we measure the skill: review problems, the recall metric and its reference configuration, the initial problem set, scoring, and the harness. |
| [`related_work.md`](related_work.md) | How the skill relates to prior art (Sashiko today; a growing survey). |

Suggested reading order is top to bottom.
[`motivation.md`](motivation.md) and [`coding-guidelines.md`](coding-guidelines.md) give the *why*;
[`interface.md`](interface.md) and [`execution-model.md`](execution-model.md) give the *what* and *how*;
[`benchmark.md`](benchmark.md) gives the *how we know it works*;
[`related_work.md`](related_work.md) situates it against prior systems.

## Design principles that recur

Three commitments cut across the documents and are worth stating once:

1. **Recall first, precision next — measured, not asserted.**
   Every design fork favours catching real defects:
   when a choice trades recall for polish,
   convenience, or determinism, recall wins today.
   Precision (raising few false alarms) is the next axis we measure and tune
   — and the benchmark, not intuition, holds the design to both.
2. **Determinism where it protects recall;
   the model only where judgement is unavoidable.**
   Change resolution and review assembly are plain scripts,
   so they are reproducible and never silently drop a finding.
   The model is used for the parts that genuinely need judgement
   — the persona passes, verification,
   consolidation, and the summary.
3. **The guidelines are the standard.**
   Subjective calls are not opinions:
   each one cites a coding-guideline rule,
   which is what lets the skill make them at all (and what distinguishes it from bug-only reviewers — see [`motivation.md`](motivation.md)).

## TODO

Two threads of work are deliberately deferred
— recorded here, with their reasoning,
so neither the intent nor the open questions are lost:

1. **Optimize for recall, then precision.**
   Drive the reference configuration (Claude Code, Opus, high effort) to **100% recall** on the benchmark by improving the coding guidelines and the harness,
   and grow the problem set as real misses surface.
   Once recall is solid, bring **precision** in as a scored axis
   — add false-positive (negative) problems and tune the skill to keep raising few false alarms (see [`benchmark.md`](benchmark.md)).
2. **First-class support for the self-review agentic loop.**
   A write → test → review loop ([`motivation.md`](motivation.md)) is the north star.
   The skill already provides the review primitives:
   the loop **commits each round** and runs `diff <base>` to review its commit series (per-commit intent and commit hygiene included),
   or `files` to review a specific uncommitted edit.
   What remains is the orchestration **outside** this review skill
   — driving the write and test steps,
   and keeping the loop's temporary/generated files out of review
   — since a full loop is more than a reviewer.
   Deferred until we have real experience running such a loop.
