# Related Work

ACR occupies the local, repository-aware end of automated code review. It
shares bug-finding goals with hosted review bots, but keeps generation,
evaluation, and publication as separate boundaries.

## Sashiko

Sashiko is an AI-assisted review system for Linux and the closest motivating
prior art. ACR makes two different choices:

- **Local runtime.** ACR can run against a checkout and write a Markdown file
  without a server or pull request. The same artifact supports a maintainer and
  an iterative development agent.
- **Guideline-backed judgement.** Asterinas has an explicit coding standard, so
  ACR may report subjective maintainability or documentation issues when it can
  ground them in a fetched guideline rule. Objective defects without a matching
  rule remain first-class and use a plain-language defect class.

These choices require stronger local mechanics: deterministic target
resolution, bounded read-only tools, persona ownership, corpus digest checks,
and an inspectable state machine. See
[Coding guidelines](coding_guidelines.md) and
[Execution model](execution_model.md).

## Hosted review workflows

Hosted systems commonly combine trigger handling, model execution, and comment
publication. ACR keeps them separate:

- the runtime produces a repository-independent report;
- the benchmark evaluates that report without exposing its answer key;
- the GitHub adapter publishes only after an explicit request.

This separation makes local reproduction and model-free testing possible and
limits credentials to the step that needs them. It does not prevent CI from
composing the pieces; it prevents CI transport concerns from becoming part of
the review contract.

