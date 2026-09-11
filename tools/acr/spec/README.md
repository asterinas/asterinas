# ACR Design Specification

This is the authoritative design of Aster Code Review (ACR),
a code-review tool for Asterinas.
It explains *why* ACR works the way it does.

## ACR in one paragraph

ACR reviews code against Asterinas's persona-keyed Coding Guidelines and writes one Markdown report.
It works in two modes, both anchored at the current checkout (`HEAD`):
`diff <base>` reviews the commits the branch added over `<base>`,
and `files` reviews the current contents of named files.
Five reviewer **personas**
— Maintainability, Development, Security, Hardware, and Documentation —
each own one part of the guidelines and review the code independently.
A Python orchestrator activates and drives them, then fact-checks, consolidates, and summarizes their findings.
Models are reached through a pluggable backend: OpenAI Agents SDK, Pi, Claude Agent SDK, or a model-free fake.

## Architecture at a glance

ACR has four layers.
Each layer depends only on the one below it,
so a new backend or tool does not change the review workflow.

```text
 run.sh | benchmark-data/run.sh       command line, profile, environment
              │  RunConfig
              ▼
 core/ orchestrator                   resolve ─► activate ─► prompts ─► review ─► assemble
              │                       ─► verify ─► consolidate ─► summarize ─► publish
              │  AgentRequest (instructions, input, output schema, tool allowlist)
              ▼
 agents/ backend                      openai-agents │ pi-agent │ claude-agent │ fake
              │  tool calls
              ▼
 tools/ ToolBroker                    read-only source, Git, and guideline tools
```

| Layer | Owns | Specified in | Code |
|---|---|---|---|
| Interface | Arguments, review modes, configuration, and the report format. | [Interface and output](interface.md) | [`run_review.py`](../src/run_review.py), [`config.py`](../src/config.py) |
| Orchestrator | The fixed stage sequence and every deterministic step. | [Execution model](execution_model.md) | [`core/`](../src/core/) |
| Backend | Running one agent on one model provider. | One document per backend, below | [`agents/`](../src/agents/) |
| Tools | What an agent may read, enforced for every backend. | [Read-only tools](execution_model.md#read-only-tools) | [`tools/`](../src/tools/) |

The [Coding Guidelines](coding_guidelines.md),
lives outside ACR and is loaded by the orchestrator and the `guideline.show` tool.

## How to read this spec

Read the documents top-down; each assumes only those above it.

| Question | Document | Answers |
|---|---|---|
| **Why?** | [Motivation and goals](motivation.md) | Why ACR exists, who it serves, and what it deliberately does not do. |
| | [Related work](related_work.md) | How ACR differs from prior review systems. |
| **What does it review against?** | [Coding guidelines](coding_guidelines.md) | Why the guidelines are organized by persona, and how a reviewer loads only the rules it needs. |
| **What does a user see?** | [Interface and output](interface.md) | The command line, the two review modes, configuration, and the report format. |
| **How does it work?** | [Execution model](execution_model.md) | The pipeline stages, persona activation, read-only tools, and what each run records. |
| | [OpenAI Agents backend](openai_agents_backend.md) | How the `openai-agents` backend uses model APIs, SDK instructions, and direct function tools. |
| | [Pi backend](pi_backend.md) | How the `pi-agent` backend isolates each agent and keeps it read-only. |
| | [Claude backend](claude_backend.md) | How the `claude-agent` backend inherits local Claude CLI configuration and uses ACR's tools. |
| **How do we know it works?** | [Benchmark and evaluation](benchmark.md) | How recall is measured, how problems are sourced without leaking answers, and how to run it. |

The three backend documents share one structure
— setup, one fresh agent per call, instructions, tools, logs —
so read the one you use and compare the others section by section.

## Design principles

Four commitments recur throughout the documents:

1. **Recall first, then precision, both measured.**
   When a design choice trades catching real defects for cost or polish, recall wins.
   The benchmark, not intuition, decides whether a change helps.
2. **Code controls the workflow; models only make judgements.**
   Target resolution, persona activation, output validation, assembly, and report writing are deterministic Python.
   Models are used only for the persona reviews, verification, consolidation, and the summary.
   This keeps runs reproducible across providers
   and reduces both time and cost.
3. **The guidelines are the standard.**
   A subjective finding must cite a guideline rule that the reviewer actually fetched.
   A concrete defect that no rule names is still reported, with a plain-language defect class.
4. **Every agent gets a narrow capability.**
   Each agent receives one bounded task, a built-in allowlist of read-only tools, and a strict output schema.
   No agent edits code or writes the report.
