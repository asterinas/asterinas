# Pi Backend

The `pi-agent` backend runs ACR's agents through Pi coding-agent SDK
instead of OpenAI Agents SDK.
Use it when your models and credentials are managed by Pi,
for example a relay provider or a Claude subscription.
It changes only how a single agent call is executed;
persona activation, scheduling, validation, assembly, and the report are the same as with any other backend.

## Setup

Use the Asterinas development Docker container;
Pi and all required dependencies are preinstalled.

1. Log in to your model provider in Pi before running ACR.
   ACR uses Pi's saved credentials and model configuration from the agent directory
   (by default `~/.pi/agent`);
   credentials do not go into the ACR profile.
2. Copy [`acr.pi.toml`](../acr.pi.toml), set `review_model` and `postprocess_model` to Pi `provider/model-id` references, and run:

   ```sh
   tools/acr/run.sh --config my-acr.pi.toml diff origin/main /tmp/review.md
   ```

The Pi-specific settings are:

```toml
[agent]
backend = "pi-agent"
review_model = "provider/model-id"       # persona reviews
postprocess_model = "provider/model-id"  # verification, consolidation, summary, benchmark grading

[pi]
node_command = "node"                # or ACR_PI_NODE_COMMAND
agent_dir = "~/.pi/agent"            # or ACR_PI_AGENT_DIR
bridge_startup_timeout_seconds = 30
shutdown_grace_seconds = 5
keep_native_sessions = true
trusted_extensions = []              # extra Pi extensions to load; empty by default
```

ACR fails before calling the model if the model is unknown to Pi
or Pi has no credentials for its provider.

## One fresh Pi agent per call

Every agent call (one persona, verification, consolidation, summary, or the grader)
starts its own Node process and one fresh Pi agent:

```text
Python orchestrator
   │  JSON lines over stdin/stdout
   ▼
Node bridge process (src/agents/pi/runtime/bridge.mjs)
   │  creates a fresh Pi SDK session
   ▼
one fresh Pi agent ── tool calls ──► back to Python
```

The bridge creates one Pi session and sends it review task directly.
Python controls persona scheduling.

So, with one agent per persona, each persona gets its own process and context;
with `per_persona_context = "no"`, all personas share one agent.
A retry starts a new process; no failed transcript is reused.
One process per call makes cleanup and log ownership unambiguous.

## What the Pi agent sees

The agent's system prompt is replaced entirely with:

1. the same instructions the OpenAI backend would use;
2. a short suffix requiring the final result to be submitted through `structured_output`,
   a tool whose arguments match the stage's output schema.

Pi's project instructions, global prompt files, and skills are not loaded.
Each session starts with no previous conversation.
The first user message is only the stage input: review input for a persona,
or structured findings for later stages.

## Structured output

Pi agent submits its final result through a tool named `structured_output`.
Each stage defines the required fields and the kind of data allowed in each field.
Pi validates the tool arguments before accepting them.
After completing the task,
the agent calls structured_output once to submit its result,
without calling other tools at the same time.
A successful submission ends the task;
no further model response is needed.
If the agent does not submit a result or submits more than one, the attempt fails.

Python still validates the returned value against the Pydantic schema and stage rules
before using it in the report.
An invalid result remains an error, not an empty list of findings.

## Tools and the write boundary

Pi agent gets none of Pi's built-in tools.
`read`, `bash`, `edit`, `write`, nested subagents, and supervisor or intercom tools are all disabled.

Instead, Python sends the role's ACR tools from the `[tools]` allowlist,
each with its name (for example `source_read` for `source.read`), description, and input schema.
The bridge registers them directly as Pi SDK custom tools alongside `structured_output`.
Python executes tool calls from the Pi agent,
but only if the tool is allowed for that agent.
Both backends use the same Python tool implementations.
`guideline.show` works the same way,
so the agent cannot bypass the digest and short-name checks
(see [Coding guidelines](coding_guidelines.md#loading-rules-on-demand)).
`web.search` is not available on this backend.

The agent cannot choose an output path.
Only Python writes files(include run artifacts and report).

## Bridge protocol

Python and [bridge](../src/agents/pi/runtime/bridge.mjs) exchange UTF-8 JSON objects, one per line.
The bridge sends protocol messages to Python through stdout.
Error and debug messages go to stderr and are also saved in the run log,
so they do not interfere with protocol parsing.

1. On startup, the bridge sends `ready` with the Pi version and its capabilities.
   Python stops if a required capability is missing.
2. Python sends one `run` message with the model, thinking level, prompts, output schema, timeouts, and tools.
3. While the agent works, the bridge sends progress and `tool.call` messages; Python answers each `tool.call` with a `tool.result`.
4. The bridge ends with exactly one `result` or `error`.

Each JSONL message is limited to **4 MiB (4,194,304 bytes)**.
A size limit prevents unexpectedly large prompts, tool results, or model responses
from consuming excessive memory or blocking communication between Python and the bridge.
The limit applies to the complete message, not to individual fields.
The limit is defined by `MAX_FRAME_BYTES` in both
[`client.py`](../src/agents/pi/client.py) and [`bridge.mjs`](../src/agents/pi/runtime/bridge.mjs).
Oversized messages and unknown or mismatched IDs are errors.
On cancellation, Python asks the bridge to abort the Pi session,
waits up to `shutdown_grace_seconds` for it to exit, then terminates and finally kills the process.

## Logs and usage

Progress, tool calls and results, the final reply, and diagnostics
are written to the same per-agent event and transcript files as with the OpenAI backend
(see [Execution model](execution_model.md#run-records)).
ACR saves the usage statistics provided by Pi, such as token counts, cost, and elapsed time.
Missing statistics are marked as unavailable, not zero.

| Component | Role |
|---|---|
| [`agents/pi/backend.py`](../src/agents/pi/backend.py) | Turns an ACR agent request into a bridge `run` message and the reply back into a response. |
| [`agents/pi/client.py`](../src/agents/pi/client.py) | Owns the Node process, the message stream, tool forwarding, and cleanup. |
| [`agents/pi/protocol.py`](../src/agents/pi/protocol.py) | Validates capabilities and normalizes usage. |
| [`agents/pi/runtime/bridge.mjs`](../src/agents/pi/runtime/bridge.mjs) | Creates the Pi session, registers tools, forwards tool calls to Python, and returns the structured result. |
