# Execution Model

ACR contains a fixed sequence of stages driven by the orchestrator in
[`core/orchestrator.py`](../src/core/orchestrator.py).
The workflow, its individual steps, and shared types, state, and logging live in [`core/`](../src/core/).
Deterministic code prepares and validates every input and output;
models are called only where judgement is needed.

```text
resolve target ─► activate personas ─► build prompts                     [code]
                                            │
            ┌───────────────┬───────────────┼───────────────┬───────────────┐
     Maintainability   Development      Security        Hardware     Documentation
            └───────────────┴───────────────┼───────────────┴───────────────┘
                     one agent per persona, in parallel                  [model]
                                            │
                                         assemble                        [code]
                                            │
                       verify ─► consolidate ─► summarize                [model]
                                            │
                                         publish                         [code]
```

## Stages

| Stage | What it does | Model? | Code |
|---|---|---|---|
| Resolve | Parse the arguments and turn the target into one review input. | No | [`core/resolve.py`](../src/core/resolve.py) |
| Activate | Choose personas from the reviewed paths. | No | [`core/activation.py`](../src/core/activation.py) |
| Build prompts | Combine each persona's instructions and guideline catalog with the review input. | No | [`core/prompts.py`](../src/core/prompts.py) |
| Review | Run one agent per active persona, in parallel. | Yes | [`core/scheduler.py`](../src/core/scheduler.py) |
| Assemble | Validate every finding, sort them, and drop exact duplicates. | No | [`core/assemble.py`](../src/core/assemble.py) |
| Verify | Fact-check every finding. | Yes | [`core/verify.py`](../src/core/verify.py) |
| Consolidate | Align the fixes of findings that share a root cause. | Yes | [`core/consolidate.py`](../src/core/consolidate.py) |
| Summarize | Write the report summary. | Yes | [`core/summary.py`](../src/core/summary.py) |
| Publish | Render and write the report, refusing to overwrite without `--overwrite`. | No | [`core/publish.py`](../src/core/publish.py) |

Model-backed stages automatically retry failed agent runs.
Set `retries` in the TOML profile's `[agent]` table to control the number of retries
in addition to the initial attempt; `0` disables retries.
If retries are exhausted, or a non-model stage fails,
the run stops with an error and records which stage failed.
Invalid model output is always an error;
it is never treated as "no findings".

## The review input

Target resolution prepares the review input for each mode:

- `diff` mode: `git log -p --reverse` over `merge-base(<base>, HEAD)..HEAD`, giving each commit's message and diff, oldest first;
- `files` mode: the selected lines of each file, each prefixed with its line number.

This text is saved as `artifacts/canonical-input.txt` in log dir.
Resolution also produces a list of reviewed paths used for activation.
The argument syntax is specified in [Interface and output](interface.md#reviewing-the-current-checkout).

## Persona activation

Activation is currently path-based and deterministic.
Three personas always run; path rules select the two specialists:

| Persona | Activation rule |
|---|---|
| Maintainability, Development, Security | Always on. |
| Hardware | Assembly files (`.s`, `.S`, `.asm`) or paths under an `arch/` directory. |
| Documentation | `book/`, Markdown, `.scml` files, or syscall paths. |

File contents do not affect activation.
Agent-based selection is planned but not yet implemented;
the current rules keep selection reproducible without a model call.

## One agent per persona

By default each active persona runs as a separate agent
that sees only its own instructions, its own guideline catalog, and the review input.

Separate agents keep each context small and focused.
A single agent covering all five personas would carry every catalog on every turn
and split its attention across all of them.

Setting `per_persona_context = "no"` runs one combined agent.
It reads the review input once, so it is cheaper, but it may find fewer defects.
Both fan-out and combined mode produce the same persona-tagged findings,
so remaining stages process them in the same way.

## Agent instructions

Each persona agent's instructions consist of:

1. shared [review contract](../src/prompts/personas/pass_contract.md);
2. `Repository Layout` and `Architecture Notes`, read directly from [`AGENTS.md`](../../../AGENTS.md);
3. persona's [prompt](../src/prompts/personas/);
4. persona's guideline catalog with its digest
   (see [Coding guidelines](coding_guidelines.md#loading-rules-on-demand)).

The instructions are identical across reviews and are kept separate from the review input,
which lets providers cache them.
The later stages use the prompts in [`prompts/`](../src/prompts/).
Every agent must return JSON matching a schema in [`core/contracts.py`](../src/core/contracts.py).

## Read-only tools

Agents can use these tools, all read-only and implemented in [`tools/`](../src/tools/):

| Tool | Purpose |
|---|---|
| `source.read`, `source.search`, `source.list` | Read a file, search for an exact string, list files. |
| `git.show`, `git.diff`, `git.log`, `git.blame` | Inspect Git objects and history. |
| `guideline.show` | Fetch the full text of guideline rules (see [Coding guidelines](coding_guidelines.md#loading-rules-on-demand)). |
| `web.search` | Provider-hosted web search (`openai-agents` backend only). |

Each role gets only the tools in its `[tools]` allowlist in toml profile.
In the reference toml profiles, persona agents can read source, Git history, and guidelines;
verification agent can also list files and search the web;
consolidation, summary, and the grader get no tools.
A combined agent gets the union of the persona allowlists.

Source tools can only access files within the reviewed repository.
Git tools reject options that write files or override configuration.
Tool output is treated as data to review, never as instructions.

Trusted host code can add tools with `acr.tools.register_tool_provider()`;
added tools follow the same allowlists.

## Assembly

Assembly is deterministic and does not call a model. It:

1. requires a result from every active persona;
2. rejects findings with missing, malformed, or unknown fields;
3. removes findings that are exactly identical within one persona;
4. sorts findings by persona, file, line, and grounding.

Similar findings from different personas are all kept.
Assembly never judges whether two findings mean the same thing.

## Verification, consolidation, and summary

**Verification** gives every finding exactly one verdict:

| Verdict | Meaning | Effect on the report |
|---|---|---|
| `confirmed` | The finding's key claim holds. | Kept unchanged. |
| `uncertain` | The claim could not be settled. | Kept, prefixed with `(unverified)`. |
| `refuted` | Concrete evidence shows the claim is false. | Moved to `Retracted by verification` with the reason. |

Doubt is `uncertain`, not `refuted`,
so a true finding is removed only by a confident and evidenced mistake.

**Consolidation** may rewrite the fixes of findings that share a root cause into one shared fix.
It cannot add, remove, or merge findings.

**Summary** writes the report's opening overview from the final findings.
It cannot change the findings.

## Run records

Each run gets a directory named `acr-<timestamp>-<id>` under `agent.log_root`
(default `$TMPDIR/aster-code-review/`, normally `/tmp/aster-code-review/`).
Its main contents are:

| Path | Contents |
|---|---|
| `manifest.json` | Configuration, input and prompt hashes, active personas, and guideline digests. |
| `state.json` | Progress through the stages, including the failing stage. |
| `main.jsonl`, `sdk-trace.jsonl` | Event log and trace for the whole run. |
| `usage.json` | Token usage summed over all agents. |
| `artifacts/` | The review input, prompts, each persona's raw findings, the result of each stage, and the final report. |
| `agents/personas/<persona>/` | Per-persona events, transcript, final response, and token usage. |
| `agents/<stage>/` | Corresponding records for combined review and later model-backed stages. |
| `pi-sessions/` | Pi session files (`pi-agent` backend only). |

Within an agent's directory:

| File | Contents |
|---|---|
| `events.jsonl` | Agent lifecycle, tool calls, and backend events. |
| `transcript.jsonl` | Tool events and backend-supplied reply or progress events. |
| `sdk-trace.jsonl` | OpenAI SDK spans, including model requests, responses, and SDK tool events, when tracing is enabled. |
| `response-<agent-run-id>.json` | The agent's final structured result. |
| `usage.json` | Reported token counts, cost, and timing. |

## Backends

The orchestrator talks to models only through the interface in [`agents/protocol.py`](../src/agents/protocol.py),
so it does not depend on any SDK.
[`agents/backend_factory.py`](../src/agents/backend_factory.py) selects one of:

- [`openai-agents`](../src/agents/openai_agents/backend.py): the OpenAI Agents SDK backend, see [OpenAI Agents backend](openai_agents_backend.md);
- [`pi-agent`](../src/agents/pi/backend.py): one fresh Pi SDK session per call, see [Pi backend](pi_backend.md);
- [`claude-agent`](../src/agents/claude/backend.py): one fresh Claude SDK client per call, see [Claude backend](claude_backend.md);
- [`fake`](../src/agents/fake.py): no model; returns empty results, for tests.

`review_model` selects the model for persona reviews, whether separate or combined.
`postprocess_model` selects the model for verification, consolidation, summary, and benchmark grading.
All backends follow this split; changing one setting does not change the other.

Retries work at two levels.
`retries` restarts a whole agent from scratch after a failure or invalid output.
`model_retries` retries a single failed model request on the `openai-agents` backend;
on `pi-agent` and `claude-agent`, their runtimes(PI CLI/Claude CLI) handle request retries themselves.
