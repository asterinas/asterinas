# ACR Runtime

## Overview

This directory contains the SDK-backed implementation of `aster-code-review`.
The deterministic state machine is in `core/` and `stages/`; OpenAI and Pi
provider adapters are isolated in `agents/`.

The organized runtime design is in [`spec/`](spec/); start with the
[`spec/README.md`](spec/README.md) reading guide. The specification links each
contract and design decision back to the implementation using relative paths.

## Configuration profiles

Portable provider profiles live in separate TOML files. Copy
`acr.example.toml` to create a documented custom profile. `acr.toml` is the
current relay profile and `acr.deepseek.toml` is the DeepSeek official API
profile. Select exactly one with `--config`; ACR neither scans for nor merges
neighboring TOML files. An explicitly selected path must exist. Credentials
remain environment variables named by that profile's provider section.

## Guideline and prompt sources

The `acr/` directory is self-contained application code: `scripts/`,
`prompts/`, schemas, benchmark definitions, and configuration all travel with
it. The checkout being reviewed supplies repository data only; it does not
select the coding-guideline corpus.

By default, `run.sh` uses a bundled `guideline-root/` snapshot when one is
present. Otherwise, it uses the guidelines from the Git checkout containing
the running `run.sh`, even when the current working directory is a different
historical or PR checkout. `ACR_GUIDELINE_ROOT` can explicitly select another
trusted snapshot. A standalone copy without a bundled snapshot must set that
variable.

Stable instructions for verification, consolidation, summary, and the
benchmark-only grader live under `prompts/`. Their runtime outputs remain
constrained by the Pydantic contracts in `core/contracts.py`.

## Requirements

ACR requires Python 3.11 or newer, `pydantic>=2.0,<3`, and
`openai-agents[any-llm]>=0.22,<0.23`. Benchmark commands additionally require
`pyyaml>=6.0`. These constraints are declared in `pyproject.toml` and can be
installed with any Python package installer, for example:

```sh
python3 -m pip install '/path/to/acr[benchmark]'
```

## Quick start

Run the model-free path from the repository root after installing the
dependencies. `ACR_DIR` may point to an `acr` directory copied anywhere:

```sh
ACR_DIR=/path/to/acr
export ACR_GUIDELINE_ROOT=/path/to/current/asterinas
"$ACR_DIR/run.sh" \
  files README.md /tmp/acr-review.md --backend=fake --overwrite
```

To review an older checkout with the latest guidelines, run `run.sh` from the
latest ACR checkout while keeping the older checkout as the current working
directory. The script binds guideline lookup to the ACR checkout rather than
the working directory under review.

## Providers

Production runs use `--backend=openai-agents` (or `ACR_BACKEND`) and read the
provider adapter, endpoint, and model from `acr.toml`; the key is read from the
environment variable named by `provider.api_key_env`. Two SDK model paths are
supported:

- `adapter = "openai"` uses the native OpenAI model implementation. Set
  `wire_api = "responses"` for the official Responses API.
- `adapter = "any-llm"` uses the Agents SDK Any-LLM adapter. An
  OpenAI-compatible relay can use a model such as `openai/gpt-5.5`, its custom
  `base_url`, and either `wire_api = "responses"` or
  `wire_api = "chat_completions"`, according to the relay's API support.

### Official OpenAI API

For the official OpenAI API, the relevant TOML values are:

```toml
[agent]
model = "gpt-5.5"
review_model = "gpt-5.5"
wire_api = "responses"

[provider]
adapter = "openai"
api_key_env = "OPENAI_API_KEY"
base_url_env = "OPENAI_BASE_URL"
```

### DeepSeek API

For the DeepSeek official Responses API, use the checked-in profile:

```sh
export DEEPSEEK_API_KEY='...'
./benchmark/run.sh --problem 0001 --backend openai-agents \
  --config ./acr.deepseek.toml --grade
```

The DeepSeek profile uses `deepseek-v4-flash`, `adapter = "openai"`, and
`base_url = "https://api.deepseek.com"`. Here `openai` names the SDK's native
OpenAI-compatible Responses transport; it does not select OpenAI's service.

### Third-party endpoints

For a third-party endpoint, select `adapter = "any-llm"`, put the provider
prefix in the model name when needed, and configure `base_url` and `wire_api`
in TOML. The checked-in relay example uses Responses. No provider URL or
credential is embedded in the Python adapter.

### Pi Agent SDK

Use `--backend=pi-agent --config ./acr.pi.example.toml` to run the same Python
orchestrator through Pi. Authentication, Claude subscriptions, and relay models
remain in Pi's own agent directory. Each invocation uses one fresh
`pi-subagents` child, receives only ACR's read-only ToolBroker tools, and returns
schema-validated data to Python; Pi `bash`, `write`, and `edit` are disabled.
Trusted custom Python tools can be added explicitly with
`acr.tools.register_tool_provider()` and are shared by both SDK backends.

## Agent limits and retries

`reasoning_effort` controls defect-finding persona runs. The mechanical
verification, consolidation, summary, and grader stages use the independently
configurable `postprocess_reasoning_effort`, so raising review depth does not
force every large post-processing request to use the same latency budget.
Omitting `max_turns` disables the SDK turn limit; set it to a positive integer
only when a turn cap is desired. Omitting `timeout_seconds` likewise disables
the whole-agent wall-clock limit; set it to a positive number only when a hard
limit for each attempt is desired. Model-request retries and per-tool timeouts
remain active when this whole-agent limit is disabled.

`model_retries` retries an individual SDK model request while preserving the
current agent/tool transcript. Its delay is controlled by the
`model_retry_*` settings. `retries` is separate and restarts an entire agent
attempt, so keep it low to avoid multiplying expensive full-persona runs.
Normal network, timeout, throttling, and server errors are retried. Relays that
encode a transient provider failure as HTTP 400 can list its exact error type
under `provider.retryable_http_error_types`; unrelated HTTP 400 responses still
fail immediately. `provider.response_input_exclude_fields` removes explicitly
configured relay-incompatible fields from replayed Responses items.

## Tool configuration

The `[tools]` table defines each persona and post-processing agent's tool
allowlist. Edit those arrays to change tool assembly without modifying Python;
combined review mode receives the union of the selected persona arrays. An
agent receives the SDK's hosted `WebSearchTool` when its array contains
`web.search`. The hosted tool requires `wire_api = "responses"` and provider
support for the Responses `web_search` tool, and its context size is controlled
by `web_search_context_size`. Source and Git tools remain local and read-only.
`tools_enabled` disables all configured tools globally.

## Output and tracing

On successful completion ACR prints colored, labelled paths for the review
report and GMT+8 run log. Use `ACR_FORCE_COLOR=1` when stdout is piped;
`NO_COLOR=1` disables colors unless `ACR_FORCE_COLOR=1` is set.

Each run also contains `sdk-trace.jsonl`. It is produced by the OpenAI Agents
SDK `TracingProcessor` and includes workflow/agent/generation/function spans,
model inputs and outputs, provider-returned reasoning summaries, and tool
arguments/results. Scoped copies are written to
`agents/personas/<persona>/sdk-trace.jsonl` (or `agents/<stage>/sdk-trace.jsonl`)
alongside the existing `events.jsonl` and response artifacts. The `[tracing]`
section controls whether SDK spans are enabled, whether payloads are included,
and the maximum logged string size. API keys and authorization values are
always redacted. Hidden chain-of-thought is never available to or written by
ACR; only provider-published reasoning summaries can appear.

## GitHub publication

GitHub publication remains a separate, opt-in step. The canonical script is
`scripts/post_reviews_to_github.sh`; it preserves the legacy command-line
interface and uses the colocated `scripts/parse_review.py`. Python callers can
use `acr.sinks.github.publish`, which only forwards to that script. The outer
skill path remains as a compatibility entry point.

## Benchmark

`benchmark/runner.py` runs real SDK reviews in detached worktrees;
`--backend=fake` remains available for deterministic tests. Ground
truth remains in the benchmark control plane and is never passed to the reviewer.

Benchmark artifacts are kept under `/tmp/aster-code-review/benchmarks/` by
default. A single-problem run directory is named
`run-<GMT+8 local timestamp>-<numeric problem ID>-<unique suffix>`; the
redundant numeric UTC offset and descriptive problem slug are omitted from this
user-visible directory name. Report filenames and internal run IDs retain their
timezone-aware timestamps. Set `ACR_BENCHMARK_ROOT` or pass `--work` to choose
another location.

Pi benchmark artifacts use the separate
`/tmp/aster-code-review/benchmarks-pi/` root and retain the GMT+8 offset and full
benchmark ID in `run-<timestamp>-<full-id>-<random>` directory names. Override
this root with `ACR_PI_BENCHMARK_ROOT`.
