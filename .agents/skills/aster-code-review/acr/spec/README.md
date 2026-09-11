# ACR Design Specification

This directory describes the design implemented by the ACR SDK runtime in the
parent directory. ACR reviews either a Git commit series or selected files
against Asterinas's persona-keyed coding guidelines and writes one Markdown
review report.

The specification is organized around stable contracts rather than one
particular model provider. The executable source remains authoritative when a
document and the implementation disagree.

## Reading guide

| Document | Subject |
|---|---|
| [Motivation and goals](motivation.md) | Why ACR exists, its users, goals, and non-goals. |
| [Coding guidelines](coding_guidelines.md) | Why the guideline corpus is keyed by reviewer persona and disclosed progressively. |
| [Interface and output](interface.md) | The `diff` and `files` modes, process options, configuration, and report format. |
| [Execution model](execution_model.md) | The state machine, structured contracts, tools, artifacts, tracing, and failure behavior. |
| [Pi backend](pi_backend.md) | Pi SDK subprocess protocol, `pi-subagents` isolation, prompt mapping, tools, and write boundary. |
| [Benchmark and evaluation](benchmark.md) | Detached-worktree evaluation, answer-key isolation, grading, and strict recall. |
| [Related work](related_work.md) | The design's relationship to service-based review systems. |

Read [Motivation and goals](motivation.md) and
[Coding guidelines](coding_guidelines.md) for the rationale, then
[Interface and output](interface.md) and [Execution model](execution_model.md)
for the runtime contract. Contributors changing evaluation should also read
[Benchmark and evaluation](benchmark.md).

## Implementation map

| Concern | Implementation |
|---|---|
| CLI and configuration | [`run.sh`](../run.sh), [`run_review.py`](../run_review.py), and [`config.py`](../config.py) |
| Workflow state machine | [`core/orchestrator.py`](../core/orchestrator.py) and [`core/state.py`](../core/state.py) |
| Structured stage boundaries | [`core/contracts.py`](../core/contracts.py) and [`core/invocations.py`](../core/invocations.py) |
| Target resolution and prompt construction | [`stages/resolve.py`](../stages/resolve.py), [`scripts/resolve_target.sh`](../scripts/resolve_target.sh), and [`stages/prompts.py`](../stages/prompts.py) |
| Backend selection and lifecycle | [`agents/backend_factory.py`](../agents/backend_factory.py) and [`agents/protocol.py`](../agents/protocol.py) |
| OpenAI and fake execution | [`agents/openai_agents/backend.py`](../agents/openai_agents/backend.py) and [`agents/fake.py`](../agents/fake.py) |
| Pi execution | [`agents/pi/backend.py`](../agents/pi/backend.py), [`agents/pi/client.py`](../agents/pi/client.py), and [`agents/pi/runtime/bridge.mjs`](../agents/pi/runtime/bridge.mjs) |
| Review refinement and rendering | [`stages/assemble.py`](../stages/assemble.py), [`stages/verify.py`](../stages/verify.py), [`stages/consolidate.py`](../stages/consolidate.py), [`stages/summary.py`](../stages/summary.py), and [`stages/publish.py`](../stages/publish.py) |
| Benchmark control plane | [`benchmark/runner.py`](../benchmark/runner.py), [`benchmark/grader.py`](../benchmark/grader.py), and [`benchmark/problems.yaml`](../benchmark/problems.yaml) |
| Optional GitHub publication | [`sinks/github.py`](../sinks/github.py) and [`scripts/post_reviews_to_github.sh`](../scripts/post_reviews_to_github.sh) |

## Design commitments

1. **Recall is measured.** The first quality target is finding known real
   defects. `partial` findings are diagnostic and do not count as caught.
2. **Determinism protects stage boundaries.** Target parsing, persona
   activation, schema validation, assembly, state persistence, and scoring are
   ordinary code. Models are used only where review judgement is necessary.
3. **The core is provider-neutral.** The orchestrator sends typed invocations
   through an agent protocol. OpenAI Agents SDK and Pi Agent SDK support are
   adapters, and the fake backend runs the same workflow without a model or
   network.
4. **Guidelines are the shared standard.** Each guideline-backed finding names
   a fetched rule short-name. Real defects without a matching rule remain
   reportable with a plain-language grounding.
5. **Repository access is read-only during review.** Local tools are scoped to
   source, Git history, and guideline lookup. The review workflow writes only
   its own artifacts and the requested report.
6. **Evaluation answers are control-plane data.** Benchmark defects are never
   copied into the checkout or input seen by the reviewer.
7. **Publication is separate and opt-in.** Generating a report does not require
   GitHub credentials and does not create external side effects.
8. **Model processes do not own review files.** In the Pi path, child agents
   cannot use shell or mutation tools. Only deterministic Python stages write
   parsed, validated artifacts and the final report.
