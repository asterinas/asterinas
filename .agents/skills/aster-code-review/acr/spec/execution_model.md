# Execution Model

ACR is an explicit asynchronous state machine. Deterministic stages prepare and
validate data; model-backed stages operate through typed contracts. The
orchestrator never depends on OpenAI SDK types.

```text
raw arguments
    │
    ▼
resolve target ──► activate personas ──► compile instructions/input
                                                │
                                                ▼
                                      persona fan-out or combined pass
                                                │
                                                ▼
validate + assemble ──► verify ──► consolidate ──► summarize ──► publish file
```

The implementation entry is
[`core/orchestrator.py`](../core/orchestrator.py). Its durable stage names are
defined in [`core/state.py`](../core/state.py).

## Stage sequence

| Stage | Responsibility | Implementation |
|---|---|---|
| `RESOLVED` | Parse the raw interface and produce canonical input and metadata. | [`stages/resolve.py`](../stages/resolve.py) |
| `ACTIVATED` | Select personas from reviewed paths. | [`core/activation.py`](../core/activation.py) |
| `PROMPTS_BUILT` | Separate stable instructions from volatile review input and record hashes. | [`stages/prompts.py`](../stages/prompts.py) |
| `PASSES_RUNNING` | Run persona invocations concurrently, or one combined invocation. | [`core/scheduler.py`](../core/scheduler.py) |
| `PASSES_VALIDATED` / `ASSEMBLED` | Validate strict comments, order them, and remove only exact within-persona duplicates. | [`stages/assemble.py`](../stages/assemble.py) |
| `VERIFIED` | Fact-check each finding and preserve explicit retractions. | [`stages/verify.py`](../stages/verify.py) |
| `CONSOLIDATED` | Coordinate fixes that share a root cause. | [`stages/consolidate.py`](../stages/consolidate.py) |
| `SUMMARIZED` | Produce a constructive synthesis of the final document. | [`stages/summary.py`](../stages/summary.py) |
| `PUBLISHED` | Render and write the requested Markdown file with overwrite protection. | [`stages/publish.py`](../stages/publish.py) |

Any exception records `FAILED` state and the error before it propagates. Invalid
model output is an error, never an implicit empty finding list.

## Canonical review input

Target resolution makes later stages independent of the selected mode:

- diff mode emits each branch commit's message and patch from merge-base to
  HEAD;
- file mode emits numbered source lines for each selected whole file or range.

The resolver writes this input to `artifacts/canonical-input.txt` and records
its SHA-256 digest in the manifest. It also determines the reviewed path set
used by activation. Full syntax is specified in
[Interface and output](interface.md#head-is-the-head).

## Persona activation

Activation is deterministic and recall-oriented:

- Maintainability, Development, and Security activate for code;
- Hardware activates for assembly, architecture paths, or source containing
  `asm!` or `global_asm!`;
- Documentation activates for book and Markdown content, SCML files, and
  syscall-facing paths.

The policy in [`core/activation.py`](../core/activation.py) preserves a stable
persona order. It does not ask a model to triage because a mistaken exclusion
would be a silent recall loss.

## Prompt and guideline construction

Each persona invocation has two separately hashed parts:

1. stable instructions: shared pass contract, SDK execution rules, persona
   template, and the persona's complete gist catalog;
2. volatile input: the canonical diff or numbered file excerpts.

Separating them lets the backend and traces identify each part independently
and enables provider-side prompt caching. The catalog digest pins exact rule
queries. See [Coding guidelines](coding_guidelines.md#progressive-disclosure).

## Provider-neutral invocations

[`core/invocations.py`](../core/invocations.py) represents instructions, input,
output type, tool policy, run limits, role, persona, and model without importing
an SDK. [`agents/protocol.py`](../agents/protocol.py) defines the backend
interface and response/event types.

Three backends implement that boundary, selected centrally by
[`agents/backend_factory.py`](../agents/backend_factory.py):

- [`agents/fake.py`](../agents/fake.py) provides deterministic, model-free
  execution for tests and smoke runs;
- [`agents/openai_agents/backend.py`](../agents/openai_agents/backend.py) translates invocations
  into OpenAI Agents SDK agents, structured outputs, tools, retries, and traces.
- [`agents/pi/backend.py`](../agents/pi/backend.py) translates each invocation
  into one isolated Pi SDK bridge process and one fresh `pi-subagents` child.

The Pi parent session performs no model turn. It only owns the Pi extension
runtime and deterministic structured delegation. Personas, verification,
consolidation, summary, and the benchmark grader are separate child
invocations; combined review mode deliberately remains one child invocation.
See [Pi backend](pi_backend.md).

OpenAI model-request retries preserve the current transcript. Provider-level Pi
retry behavior remains owned by Pi. ACR agent retries restart a whole persona
or post-processing attempt for either backend. A configured agent timeout
bounds an attempt; omitting it allows the attempt to complete. Persona review
uses `reasoning_effort`, while verification, consolidation, summary, and
grading use `postprocess_reasoning_effort`.

## Fan-out and combined mode

With `per_persona_context = "auto"` or `"yes"`, the scheduler runs one isolated
invocation per activated persona, bounded by `max_concurrency`. Each receives
only its persona instructions and the shared canonical input. This is the
recall-first default.

With `"no"`, prompt construction combines all activated persona blocks and the
scheduler makes one invocation. This pays the input and model startup cost once
but increases the amount of review responsibility in one context.

Both paths return the same persona-tagged `CommentsEnvelope`, so downstream
validation and assembly are identical.

## Read-only tools

The default broker in [`tools/builtin.py`](../tools/builtin.py) exposes bounded,
read-only capabilities:

- source read, exact-string search, and file listing;
- selected Git object, history, diff, and blame operations;
- digest-checked exact guideline lookup.

Repository paths are confined below the reviewed repository root, and dangerous
Git configuration/output switches are rejected. Not every registered tool is
available to every role: the `[tools]` table in
[`acr.toml`](../acr.toml) defines each agent's allowlist, and
[`agents/factories.py`](../agents/factories.py) applies it. Combined review mode
receives the union of its persona allowlists. Hosted web search is assembled
only by the OpenAI adapter when the agent's allowlist contains `web.search`.
The Pi adapter exposes only specs present in the Python broker; unmatched
allowlist entries grant no child capability.

Repository and tool output is untrusted review data, not instructions.

Explicitly trusted applications may register another provider with
`acr.tools.register_tool_provider()`. It is merged into the same
`CompositeToolProvider`, so OpenAI and Pi consume one tool contract and one
allowlist. Pi accepts only the adapter's recognized read-only capability
classes and independently denies shell, mutation, and nested-agent names.

## Assembly

Persona output is parsed against Pydantic contracts in
[`core/contracts.py`](../core/contracts.py). Assembly:

1. requires a fragment for every activated persona;
2. rejects malformed or unknown fields;
3. removes byte-equivalent structured duplicates only within one persona;
4. sorts by persona order, file, line, grounding, and problem text;
5. preserves all non-identical cross-persona findings.

This stage does not use a model and does not make semantic deduplication
decisions.

## Verification, consolidation, and summary

### Verification

Verification isolates each comment's load-bearing premise and returns exactly
one `confirmed`, `uncertain`, or `refuted` item per comment.

- confirmed findings remain unchanged;
- uncertain findings remain and are visibly prefixed `(unverified)`;
- refuted findings leave the main sections but remain in the report's retraction
  section with a reason.

Only confident refutation removes a finding from the main body.

### Consolidation

Consolidation may define shared fixes and update individual `Fix.` text when
several symptoms share a remedy. It cannot add or delete comments. Unknown
comment identifiers fail validation.

### Summary

The summary agent sees the final structured document and returns one non-empty
Markdown synthesis. It cannot mutate the comment collection.

The stable role prompts are stored in [`prompts/`](../prompts/) and the output
schemas in [`core/contracts.py`](../core/contracts.py).

## Run state, artifacts, and tracing

Every run receives a unique GMT+8 ID below `agent.log_root` (normally
`/tmp/aster-code-review/`). The directory contains:

```text
manifest.json
state.json
main.jsonl
sdk-trace.jsonl
usage.json
artifacts/
  canonical-input.txt
  instructions/
  inputs/
  prompts/
  fragments/
  assembled.json
  verified.json
  consolidated.json
  summary.md
  final-review.md
agents/
  personas/<persona>/
    events.jsonl
    transcript.jsonl
    response-<agent-run-id>.json
    usage.json
  <post-processing-stage>/
    events.jsonl
    transcript.jsonl
    response-<agent-run-id>.json
    usage.json
pi-sessions/
```

The manifest records configuration, input and prompt hashes, activated
personas, catalog digests, contract version, and non-secret Pi runtime metadata
when applicable. State and JSON artifacts are written atomically where
applicable. Event logs redact credentials and bound string size. Terminal
events and `usage.json` preserve reported input, output, cache, turn, tool, and
duration fields; missing provider usage is marked unreported rather than
invented as zero.

The OpenAI adapter's local SDK tracing processor records workflow, agent,
generation, and tool spans. The Pi adapter instead projects bridge progress,
tool RPC, assistant reply, terminal usage, and diagnostics into the common
event and transcript streams. Payload inclusion and maximum size are
configurable. ACR can record only reasoning summaries returned by a provider;
hidden chain-of-thought is not available.

## Publication boundary

The workflow's final `publish` stage writes only the requested local Markdown
file. External GitHub publication is not part of the state machine and requires
a separate explicit call. This keeps model credentials out of the publisher
and GitHub credentials out of ordinary review generation.
