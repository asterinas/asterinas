# Aster Code Review (ACR)

ACR is a local **code-review CLI** for the Asterinas OS kernel.
It reviews a Git commit series (`diff` mode) or selected working-tree files (`files` mode)
against Asterinas's persona-keyed [Coding Guidelines](../../../../book/src/to-contribute/coding-guidelines/)
and writes one Markdown report with findings and suggested fixes.
It covers maintainability, correctness, security, hardware behavior, and documentation.

It is **provider-neutral**: the same review workflow runs through OpenAI Agents SDK or Pi Agent SDK.
It is **local-first**: it reads a checkout and writes a local report; no server or PR is required.
It is **benchmark-driven**: known defects measure recall, so review quality is tested against evidence.

## Quick start

Run these commands from the root of the Asterinas checkout you want to review.
The checked-in `run.sh` loads ACR's source from this checkout.

Set an API key for the [OpenAI example profile](acr.example.toml),
then review the commits on your branch since its merge base with `origin/main`:

```sh
export OPENAI_API_KEY='...'
.agents/skills/aster-code-review/acr/run.sh \
  --config .agents/skills/aster-code-review/acr/acr.example.toml \
  diff origin/main /tmp/acr-review.md
```

To review working-tree files, including uncommitted edits:

```sh
.agents/skills/aster-code-review/acr/run.sh \
  --config .agents/skills/aster-code-review/acr/acr.example.toml \
  files README.md kernel/src/lib.rs:1-80 /tmp/acr-files.md
```

`diff <base>` reviews the commits in `merge-base(<base>, HEAD)..HEAD`;
it does not include uncommitted edits.
`files <path[:lines] ...>` reviews current file contents;
line numbers are 1-based and ranges are inclusive.
The last argument is the report path.
Add `--overwrite` to replace an existing report.
See the [interface spec](spec/interface.md) for the full target syntax and options.

## Configuration

Select one TOML profile with `--config`.
Copy `acr.example.toml` to `acr.tmol`, and modify the configuration.

The profile sets the model, agent limits, tool permissions, and provider options.
`--backend` can select `openai-agents`, `pi-agent`, or the model-free `fake` backend.
Environment variables such as `ACR_MODEL` override profile values;
process options take precedence over both.
See the profile comments and [configuration spec](spec/interface.md#configuration) for details.

## What's in this directory

| Path | Purpose |
|---|---|
| [`run.sh`](run.sh) | Shell entry point for the review. |
| [`core/`](core/) and [`stages/`](stages/) | Review workflow, validation, and report assembly. |
| [`agents/`](agents/) | OpenAI, Pi, and model-free backend adapters. |
| [`tools/`](tools/) and [`prompts/`](prompts/) | Scoped review tools and agent instructions. |
| [`benchmark/`](benchmark/) | Known-defect evaluation and its runner. |
| [`tests/`](tests/) | Model-free checks of review workflow. |
| [`spec/`](spec/) | The design specification. Start at [`spec/README.md`](spec/README.md). |

## Design specification

The [specification](spec/README.md) explains the motivation,
coding guidelines, interface, execution model, and benchmark.

## Reports and benchmark

The report has a summary and findings grouped by reviewer persona.
[Report format](spec/interface.md#markdown-report) describes the fields.

The [benchmark](benchmark/README.md) checks ACR against known defects in detached worktrees.
With `--grade`, it counts fully caught defects toward strict recall
and reports partial matches separately:

```sh
.agents/skills/aster-code-review/acr/benchmark/run.sh \
  --problem 0001 --backend openai-agents \
  --config .agents/skills/aster-code-review/acr/acr.example.toml --grade
```

See the [benchmark spec](spec/benchmark.md) for scoring and isolation details.