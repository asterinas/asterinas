# OpenAI Agents Backend

The `openai-agents` backend runs ACR's agents through the Python OpenAI Agents SDK.
ACR owns persona activation, scheduling, whole-agent retries, validation, and report generation.
The adapter lives in [`src/agents/openai_agents/`](../src/agents/openai_agents/).

## What's in this directory

| Component | Role |
|---|---|
| [`agents/openai_agents/backend.py`](../src/agents/openai_agents/backend.py) | Creates the model client, SDK agent, Runner invocation, output schema, and request retry policy. |
| [`agents/openai_agents/protocol.py`](../src/agents/openai_agents/protocol.py) | Defines backend errors and adapts provider errors, final output, usage, and reasoning summaries. |
| [`agents/openai_agents/responses.py`](../src/agents/openai_agents/responses.py) | Removes unknown metadata from replayed Responses messages using the SDK's protocol schema. |
| [`agents/openai_agents/tracing.py`](../src/agents/openai_agents/tracing.py) | Routes SDK traces and spans to ACR's local event logger. |
| [`agents/openai_agents/tools.py`](../src/agents/openai_agents/tools.py) | Adapts repository tools into SDK `FunctionTool` callbacks. |
| [`tools/protocol.py`](../src/tools/protocol.py) | Defines the shared broker and enforces tool allowlists, timeouts, and output limits for all backends. |

## How to use this backend

Provide API key and BaseURL through the environment:

```sh
export OPENAI_API_KEY='<your-api-key>'
export OPENAI_BASE_URL='<your-base-url>'
tools/acr/run.sh --config tools/acr/acr.toml diff origin/main /tmp/review.md
```

Edit [`acr.toml`](../acr.toml) to select the adapter, API, model names,
and the environment-variable names used for authentication and routing.
The key and endpoint values come only from those environment variables;
the profile contains no literal endpoint URL.
The profile uses the `any-llm` adapter for third-party provider;
for the official OpenAI API, set `provider.adapter = "openai"`
and use model IDs accepted by that API.
An official-API profile can contain:

```toml
[agent]
backend = "openai-agents"
review_model = "gpt-5.5"
postprocess_model = "gpt-5.5"
wire_api = "responses"

[provider]
adapter = "openai"
api_key_env = "OPENAI_API_KEY"
base_url_env = "OPENAI_BASE_URL"
```

## One fresh SDK agent per call

Each `AgentBackend.run()` creates a fresh SDK `Agent`, model client, and `Runner.run()` invocation.
The Runner handles model responses and tool calls until it gets the stage's final output.

For fanout mode, ACR passes no conversation history or SDK session between personas.
One persona's transcript therefore doesn't become another persona's context.
Combined mode receives one invocation containing all active persona blocks.

## Instructions and input

See [Agent instructions](execution_model.md#agent-instructions)
for the shared instruction contents and stage-specific prompts.

The backend supplies the instructions as `Agent.instructions`
and passes task input separately to `Runner.run()`.

## Tools and the write boundary

[`agents/openai_agents/tools.py`](../src/agents/openai_agents/tools.py)
converts the role-scoped broker's definitions into SDK `FunctionTool` objects for `Agent.tools`.
Their Python callbacks execute through the shared `ToolBroker`, without an MCP transport.
The broker enforces allowlists, timeouts, and output limits;
the default repository tools are read-only.

## Logs and cost

Agent and broker events, final output, returned reasoning summaries, and token usage
use the common [run records](execution_model.md#run-records).