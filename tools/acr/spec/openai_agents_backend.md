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

Export the API key and, for a relay, its endpoint, then run:

```sh
export ACR_API_KEY='<your-api-key>'
export ACR_BASE_URL='<your-base-url>'   # omit for the official OpenAI API
tools/acr/run.sh diff origin/main /tmp/review.md
```

Without `--config`, ACR uses [`acr.openai.toml`](../acr.openai.toml), the profile for this backend.
Its `[openai]` table selects the adapter and the wire API.
The reference profile targets a third-party relay through the `any-llm` adapter,
with models named `provider/model-id`.
For the official OpenAI API, use a profile such as:

```toml
[agent.persona_review]
model = "gpt-5.5"

[agent.postprocess]
model = "gpt-5.5"

[openai]
adapter = "openai"
```

See [Configuration](interface.md#configuration) for every setting.

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

## Web search

With `openai.wire_api = "responses"`, Maintainability, Development, and verification
may use the provider's hosted web search (`web.search`).
Not every provider or relay supports it, so ACR checks before the review starts.
For each distinct model and effort among those roles,
`web_search_refusal()` sends one tiny request carrying the web-search tool.
If the provider rejects it, ACR prints `web search disabled for model ...` with the reason,
removes `web.search` from the roles that use that model, and continues.
Each probe is recorded as a `web_search.probe` event in `main.jsonl`,
and the manifest records the tools that were finally offered.

## Logs and cost

Agent and broker events, final output, returned reasoning summaries, and token usage
use the common [run records](execution_model.md#run-records).