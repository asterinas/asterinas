# Claude Backend

`claude-agent` backend runs ACR through Claude Agent SDK
and local Claude CLI executable on `PATH`.
It inherits that CLI's authentication and connection settings.
ACR continues to own persona scheduling, retries, tool authorization, validation, and report generation.
Claude Agent SDK integration stays in [`src/agents/claude/`](../src/agents/claude/).

## What's in this directory

| Component | Role |
|---|---|
| [`agents/claude/backend.py`](../src/agents/claude/backend.py) | Creates the SDK client, supplies instructions and the output schema, consumes responses, and emits ACR events. |
| [`agents/claude/cli.py`](../src/agents/claude/cli.py) | Finds the local Claude CLI on `PATH` and redacts credentials from diagnostics. |
| [`agents/claude/protocol.py`](../src/agents/claude/protocol.py) | Defines backend errors, validates structured output, and normalizes usage. |
| [`agents/claude/session.py`](../src/agents/claude/session.py) | Manages the SDK connection, timeouts, cancellation, and subprocess cleanup. |
| [`agents/claude/tools.py`](../src/agents/claude/tools.py) | Exposes role-scoped ACR tools through the SDK-managed MCP server and validates the runtime's tool allowlist. |

## How to use this backend

By default, ACR uses the local Claude CLI's authentication and connection settings, whether configured for a Claude subscription, the official API, or a compatible third-party API.

Then run:

```sh
tools/acr/run.sh --config tools/acr/acr.claude.toml diff origin/main /tmp/review.md
```

No changes to [`acr.claude.toml`](../acr.claude.toml) are needed for authentication.
ACR explicitly selects Claude CLI from `PATH`.

## Session and tool isolation

Each `AgentBackend.run()` creates a new `ClaudeSDKClient`,
sends one task through `client.query()`, consumes `receive_response()`, and disconnects.

```text
Python orchestrator
   │  creates one client for the task
   ▼
Claude Agent SDK (ClaudeSDKClient)
   │  starts claude from PATH; JSON messages over stdin/stdout
   ▼
Claude Code subprocess ── MCP tool calls ──► Python SDK MCP server ──► ToolBroker
   │  model requests
   ▼
Model service (using the local Claude CLI's connection settings)
```

Claude CLI owns the model-and-tool loop; Python controls persona scheduling and the review stages.
ACR starts the CLI in safe mode, disables skills and native tools,
and restricts MCP servers to tools supplied by ACR.

## Instructions and input

See [Agent instructions](execution_model.md#agent-instructions)
for the shared instruction contents and stage-specific prompts.

The backend passes the instructions as a string in `ClaudeAgentOptions.system_prompt`.
The SDK forwards it as `--system-prompt`, replacing Claude Code's default system prompt.
The task input is sent separately through `client.query()`.

## Logs and cost

Agent replies, broker tool events, structured results, and usage
use the common [run records](execution_model.md#run-records).

ACR records the cumulative usage returned in each SDK result;
missing statistics remain unavailable.
Reported usage from failed attempts also contributes to the run totals.
`total_cost_usd` is the runtime's reported cost,
which may differ from actual subscription.
