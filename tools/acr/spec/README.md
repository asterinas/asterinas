# ACR Design Specification

This is the authoritative design of Aster Code Review (ACR),
a code-review tool for Asterinas.
It explains *why* ACR works the way it does.

## ACR in one paragraph

ACR reviews code against Asterinas's persona-keyed Coding Guidelines and writes one Markdown report.
It works in two modes, both anchored at the current checkout (`HEAD`):
`diff <base>` reviews the commits the branch added over `<base>`,
and `files` reviews the current contents of named files.
There are five reviewer **personas** used by ACR
— Maintainability, Development, Security, Hardware, and Documentation —
each own one part of the guidelines and review the code independently.
A Python orchestrator activates and drives them, then fact-checks, consolidates, and summarizes their findings.
Models are reached through a pluggable backend: OpenAI Agents SDK, Pi, Claude Agent SDK, or a model-free fake.

## How to read this spec

| Document | Answers |
|---|---|
| [Motivation and goals](motivation.md) | Why ACR exists, who it serves, and what it deliberately does not do. |
| [Coding guidelines](coding_guidelines.md) | Why the guidelines are organized by persona, and how a reviewer loads only the rules it needs. |
| [Interface and output](interface.md) | The command line, the two review modes, configuration, and the report format. |
| [Execution model](execution_model.md) | The pipeline stages, persona activation, read-only tools, and what each run records. |
| [OpenAI Agents backend](openai_agents_backend.md) | How `openai-agents` backend uses model APIs, SDK instructions, and direct function tools. |
| [Pi backend](pi_backend.md) | How `pi-agent` backend isolates each agent and keeps it read-only. |
| [Claude backend](claude_backend.md) | How the `claude-agent` backend inherits local Claude CLI configuration and uses ACR's tools. |
| [Benchmark and evaluation](benchmark.md) | How recall is measured, how problems are sourced without leaking answers, and how to run it. |
| [Related work](related_work.md) | How ACR differs from prior review systems. |

Read the motivation and guidelines for the *why*,
the interface, execution model, and relevant backend for the *what* and *how*,
and the benchmark for *how we know it works*.

## Design principles

Four commitments recur throughout the documents:

1. **Recall first, then precision, both measured.**
   When a design choice trades catching real defects for cost or polish, recall wins.
   The benchmark, not intuition, decides whether a change helps.
2. **Code controls the workflow; models only make judgements.**
   Target resolution, persona activation, output validation, assembly, and report writing are deterministic Python.
   Models are used only for the persona reviews, verification, consolidation, and the summary.
   This keeps runs reproducible across providers,
   and reduce both time and cost expenditures.
3. **The guidelines are the standard.**
   A subjective finding must cite a guideline rule that the reviewer actually fetched.
   A concrete defect that no rule names is still reported, with a plain-language defect class.
4. **Every agent gets a narrow capability.**
   Each agent receives one bounded task, an allowlist of read-only tools, and a strict output schema.
   No agent edits code or writes the report.
