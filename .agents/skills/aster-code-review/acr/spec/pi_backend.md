# Pi Agent SDK Backend

The Pi backend is an adapter beneath ACR's existing Python orchestrator. It
does not replace persona activation, scheduling, stage transitions, Pydantic
contracts, deterministic assembly, publication, or benchmark scoring.

Its implementation is split by runtime boundary:

| Component | Responsibility |
|---|---|
| [`agents/pi/backend.py`](../agents/pi/backend.py) | Convert one provider-neutral `AgentRequest` into a Pi run request and map terminal data back to `AgentResponse`. |
| [`agents/pi/client.py`](../agents/pi/client.py) | Own one Node subprocess, validate JSONL framing and handshake capabilities, service reverse tool RPC, and enforce cleanup. |
| [`agents/pi/protocol.py`](../agents/pi/protocol.py) | Validate protocol capabilities and normalize provider-reported usage without inventing missing values. |
| [`agents/pi/runtime/bridge.mjs`](../agents/pi/runtime/bridge.mjs) | Load the installed Pi SDK and `pi-subagents`, create the parent session, register one runtime child, and perform structured delegation. |
| [`agents/pi/runtime/tool_extension.ts`](../agents/pi/runtime/tool_extension.ts) | Register the invocation's ACR tools inside the child and proxy executions to Python. |

## Control flow

Each `AgentBackend.run()` call has an isolated lifecycle:

```text
Python orchestrator
    │ AgentInvocation / AgentRequest
    ▼
PiAgentBackend
    │ versioned JSONL over stdin/stdout
    ▼
one Node bridge process
    │ Pi in-process extension event bus
    ▼
pi-subagents structured delegation
    │
    ▼
one fresh Pi child agent
```

The bridge creates a Pi parent `AgentSession` only to host extensions and the
event bus. The parent sends no user prompt and performs no model request. After
extension binding, the bridge registers a unique runtime agent through
`pi-subagents:runtime-agent-register:v1`, then emits one structured delegation
request. The child makes the model calls.

Persona fan-out is still owned by [`core/scheduler.py`](../core/scheduler.py).
With isolated persona context, concurrent Python calls therefore produce
separate bridge processes and fresh children. Verification, consolidation,
summary, and benchmark grading also use fresh children. Combined mode is the
intentional exception: all activated persona instructions are compiled into
one invocation and one child, after which Python separates comments by their
`persona` field.

Whole-agent retries repeat this lifecycle from the beginning. No failed
child transcript becomes another attempt's context.

## Prompt mapping

The child runtime uses `systemPromptMode: "replace"` and disables project,
global, and skill inheritance. Its system prompt is:

1. the exact `AgentInstructions.text` already used as OpenAI agent
   instructions;
2. an ACR transport suffix requiring exactly one JSON value matching the
   canonical, minified Pydantic JSON Schema.

The first child user prompt is only `AgentInput.content`: canonical diff or
numbered file input for review, or the existing structured stage input for
verification, consolidation, summary, and grading. Instructions are not copied
into the user prompt, and no parent transcript is inherited.

The replacement applies to ambient Pi prompt context. Pi's internal child
runtime may still install its structured-output protocol tool, which is a
transport mechanism rather than ACR review policy.

## Structured output

The delegation request uses:

```json
{
  "result": {
    "kind": "structured",
    "schema": {}
  }
}
```

`pi-subagents` captures and validates the child's `structured_output` value.
The bridge accepts only a completed structured response and returns the JSON
value to Python. Existing Python parsing and semantic checks remain the trusted
boundary before artifacts or the report are written. Provider-side validation
reduces malformed output; it does not replace `parse_comments()`,
`parse_model()`, or stage-specific invariants.

## Tool injection and execution

Pi uses ACR's existing [`ToolBroker`](../tools/protocol.py); it does not expose
Pi's built-in repository or shell tools.

For each invocation, Python narrows the broker to the role's `[tools]`
allowlist and sends these fields for every surviving tool:

- canonical dotted name, such as `source.read`;
- model-facing legal name, such as `source_read`;
- description;
- JSON input Schema.

The runtime child definition activates only the legal names and loads the ACR
tool extension. `pi.registerTool()` supplies each description and Schema to the
model as a provider tool definition; they are not manually concatenated to the
system or user prompt. On execution, the extension emits a `tool.call` frame,
Python rechecks the canonical name against the scoped broker, invokes it with
its timeout and output bound, and returns a `tool.result` frame.

This route also implements progressive disclosure:

```text
Pi guideline_show
  -> child tool extension
  -> reverse JSONL RPC
  -> Python ToolBroker
  -> guideline.show
  -> core.disclosure
  -> scripts/print_guideline.py
```

The model cannot select the guideline script path or bypass persona, digest,
and short-name validation.

Trusted host code may call `acr.tools.register_tool_provider()` before backend
creation. Registered providers are merged with the default provider and are
therefore available to both model adapters only when the role allowlist names
them. Pi additionally accepts only recognized read-only capability classes.
There is no repository-driven provider discovery or import.

## Capability and write boundary

The child definition explicitly excludes Pi's `read`, `bash`, `edit`, `write`,
and `powershell` tools, along with `subagent`, `contact_supervisor`, and
`intercom`. Nested subagents are disabled. The only active ACR tools are the
scoped, read-only definitions plus `pi-subagents`' internal structured-output
mechanism.

Neither the model nor its structured value selects an output path. The write
path is always:

```text
structured child value
  -> Python JSON decode
  -> Pydantic validation
  -> deterministic stage validation
  -> RunContext artifact or publish()
```

Pi session and observation files are host-owned runtime artifacts, not model
write capabilities. The Python client reserves a per-invocation directory
below the current ACR run root and points Pi's default session root there; the
directory may remain empty when the selected Pi storage mode is in-memory.

## Authentication and model selection

Pi authentication remains owned by Pi. ACR does not read an OpenAI key for
this backend and does not store a relay token or Claude credential in its TOML.
The bridge creates Pi's `ModelRuntime` using `auth.json` and `models.json` from
`pi.agent_dir`, resolves the configured `provider/model-id`, and fails before
delegation when the model or provider authentication is unavailable.

The relevant configuration is:

```toml
[agent]
backend = "pi-agent"
model = "provider/model-id"
review_model = "provider/model-id"

[pi]
node_command = "node"
agent_dir = "~/.pi/agent"
bridge_startup_timeout_seconds = 30
shutdown_grace_seconds = 5
keep_native_sessions = true
trusted_extensions = []
```

`ACR_PI_NODE_COMMAND` and `ACR_PI_AGENT_DIR` provide local overrides. Trusted
native extensions are an explicit host allowlist and default to empty. They do
not weaken ACR's Python ToolBroker policy for proxied tools.

## JSONL bridge protocol

Frames are UTF-8, LF-delimited JSON objects with `version = 1`. Standard output
is reserved for protocol frames; bridge and SDK diagnostics are redirected to
standard error and projected into events.

The bridge sends `ready` first, including Pi and `pi-subagents` versions and
the capabilities required by Python:

- structured delegation;
- runtime agent registration;
- terminal usage;
- reverse tool proxy;
- cancellation.

Python sends one `run` frame containing role, persona, cwd, model, thinking
level, system prompt, user prompt, output Schema, timeouts, tools, and trusted
extensions. During delegation the bridge may send progress and tool frames.
Exactly one terminal `result` or `error` belongs to the run ID. Frames are size
bounded, and unknown or mismatched tool/run identities fail closed.

Cancellation emits the exact delegation identity before the Python client
waits for graceful exit, terminates the process, and finally kills it if the
configured grace period expires. One process per invocation makes cleanup and
event ownership unambiguous.

## Observability

Bridge frames carry a source UTC timestamp; Python adds the authoritative
GMT+8 receipt timestamp, run and benchmark identities, role, persona, and
agent-run identity. Per-agent event and transcript streams contain progress,
tool arguments/results, structured replies, terminal status, and bounded
diagnostics. Response artifacts preserve the parsed final value.

When `pi-subagents` reports usage, ACR records input, output, cache read/write,
turn, tool-call, cost, model-duration, startup-duration, and total-duration
fields. Per-agent and run-level `usage.json` files retain both totals and
coverage. Missing usage remains explicitly unreported.

Pi benchmark directories use
`/tmp/aster-code-review/benchmarks-pi/run-<GMT+8>-<full-id>-<random>/` by
default, independently of OpenAI benchmark storage. See
[Benchmark and evaluation](benchmark.md).
