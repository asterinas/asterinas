# Motivation and Goals

## Why ACR exists

AI-assisted development raises code throughput, but maintainers remain
responsible for accepting kernel changes. ACR carries the broad,
checklist-driven first review pass so human attention can stay on design and
acceptance decisions.

The same reviewer is useful in an automated write-test-review loop. A local
program can review each committed iteration with `diff <base>`, or audit a
specific uncommitted file with `files`. Its Markdown report is both a human
artifact and a machine-readable feedback signal. See
[Interface and output](interface.md).

Review is not limited to new changes. File mode can audit existing code for
latent correctness, security, hardware, maintainability, or documentation
problems.

## Goals

- **High recall.** Surface real guideline violations, correctness defects,
  security and soundness problems, and hardware or ABI hazards.
- **Useful precision.** Verification should remove only confidently refuted
  findings and retain uncertainty explicitly.
- **Grounded, actionable reports.** Every comment identifies a concrete
  problem, assigns a severity, and proposes a fix. Guideline-backed comments
  cite a rule short-name.
- **Reproducible mechanics.** Parsing, activation, contracts, assembly,
  persistence, and benchmark accounting are deterministic and testable without
  a model.
- **Portable model execution.** Provider and wire-protocol details stay behind
  the agent adapter and configuration boundary.
- **Observable runs.** Manifests, stage state, backend-neutral event logs,
  artifacts, transcripts, usage accounting, and available SDK traces make a
  run inspectable after success or failure.
- **Evidence-based evolution.** The benchmark in
  [Benchmark and evaluation](benchmark.md) measures known-defect recall rather
  than treating prompt quality as an intuition.

## Non-goals

- ACR is not a formatter, compiler, or lint replacement. Existing deterministic
  tools should continue to enforce issues they can decide more reliably.
- ACR does not accept a change on behalf of a maintainer.
- The review workflow is not intrinsically a GitHub bot. GitHub publication is
  a separate, explicit adapter.
- ACR does not execute code from the repository merely to review it. Its model
  tools are read-only repository and history interfaces.
- The benchmark is not part of the reviewer. It is an isolated control plane
  with access to the expected defects only after review generation.

## Two consumers, one artifact

The canonical result is a Markdown file:

- maintainers can read it locally or explicitly publish it to a pull request;
- an iterative development agent can treat each finding's `Fix.` paragraph as
  the next round's work list.

Keeping the artifact independent of the transport avoids coupling review
quality to GitHub and keeps local and automated use on the same code path.
