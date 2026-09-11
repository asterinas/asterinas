# Pi Backend

The `pi-agent` backend runs ACR's agents through Pi coding-agent SDK
instead of OpenAI Agents SDK.
Use it when your models and credentials are managed by Pi,
for example a relay provider or a Claude subscription.
It changes only how a single agent call is executed;
persona activation, scheduling, validation, assembly, and the report are the same as with any other backend.

## What's in this directory

| Component | Role |
|---|---|
| [`agents/pi/backend.py`](../src/agents/pi/backend.py) | Turns an ACR agent request into a bridge `run` message and the reply back into a response. |
| [`agents/pi/client.py`](../src/agents/pi/client.py) | Owns the Node process, the message stream, tool forwarding, and cleanup. |
| [`agents/pi/protocol.py`](../src/agents/pi/protocol.py) | Validates capabilities and normalizes usage. |
| [`agents/pi/runtime/bridge.mjs`](../src/agents/pi/runtime/bridge.mjs) | Creates the Pi session, registers tools, forwards tool calls to Python, and returns the structured result. |

## How to use this backend

Use the Asterinas development Docker container;
Pi and all required dependencies are preinstalled.

1. Log in to your model provider in Pi before running ACR.
   ACR uses Pi's saved credentials and model configuration from the agent directory
   (by default `~/.pi/agent`);
   credentials do not go into the ACR profile.
2. Edit [`acr.pi.toml`](../acr.pi.toml), set `review_model` and `postprocess_model` to Pi `provider/model-id` references, and run:

```sh
tools/acr/run.sh --config tools/acr/acr.pi.toml diff origin/main /tmp/review.md
```

A minimal model selection contains:

```toml
[agent]
backend = "pi-agent"
review_model = "provider/model-id"       # persona reviews
postprocess_model = "provider/model-id"  # verification, consolidation, summary, benchmark grading
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
It imports the Pi SDK library and calls `createAgentSession()`;
it does not start interactive `pi` CLI.
Python controls persona scheduling.

So, with one agent per persona, each persona gets its own process and context;
with `per_persona_context = "no"`, all personas share one agent.
A retry starts a new process; no failed transcript is reused.
One process per call makes cleanup and log ownership unambiguous.

## Instructions and input

See [Agent instructions](execution_model.md#agent-instructions)
for the shared instruction contents and stage-specific prompts.

The bridge replaces Pi's default system prompt through `DefaultResourceLoader.systemPromptOverride`,
using the shared instructions plus a short contract for submitting the result through `structured_output`.
It sends the task input separately through `session.prompt()`.
Pi's project instructions, global prompt files, and skills are not loaded.

## Tools and write boundary

Pi agent gets none of Pi's built-in tools.
`read`, `bash`, `edit`, `write`, nested subagents, and supervisor or intercom tools are all disabled.

Instead, Python sends the role's ACR tools from the `[tools]` allowlist,
each with its name (for example `source_read` for `source.read`), description, and input schema.
The bridge registers them directly as Pi SDK custom tools alongside `structured_output`.
The bridge supplies them in `createAgentSession(customTools=...)` and disables built-in tools.
Each custom tool's `execute` callback sends a `tool.call` JSONL message to Python.
Python invokes the role-scoped `ToolBroker` and returns a `tool.result` message.
The bridge also checks that the session's active tools match the expected names.
Combined mode uses the union of its personas' allowlists.

This is ACR's own bridge protocol, not MCP.
The [OpenAI backend](openai_agents_backend.md#tools-and-the-write-boundary) uses direct `FunctionTool` callbacks;
the [Claude backend](claude_backend.md#tools-and-the-write-boundary) uses an SDK-managed MCP server.
All three call the same Python repository-tool implementations through `ToolBroker`,
which enforces the allowlist, timeouts, and output limits.
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

## Logs and cost

Progress, tool calls and results, the final reply, and diagnostics
are recorded by default in the per-agent event and transcript files
(see [Execution model](execution_model.md#run-records)).
The final structured result is also saved as `response-<agent-run-id>.json`.
ACR saves the cost statistics provided by Pi, such as token counts and elapsed time.
Missing statistics are marked as unavailable, not zero.