# ACR Design Specification

This specification describes the design of Aster Code Review (ACR).
ACR reviews commit series and selected files against Asterinas's coding guidelines
and produces a Markdown report with findings and suggested fixes.

Start with [Motivation and goals](motivation.md) for the problem ACR addresses.
For commands, configuration, and examples, see the [user guide](../README.md).

## Topics

| Document | Subject |
|---|---|
| [Motivation and goals](motivation.md) | Why this tool exists, and its goals and scope. |
| [Coding guidelines](coding_guidelines.md) | The persona-keyed Coding Guidelines the skill consumes. 5 review personas, the rules they apply, and when reviewers consult them. |
| [Interface and output](interface.md) | Review targets, configuration semantics, and the report format. |
| [Execution model](execution_model.md) | Review stages, validation, tool permissions, and failure handling. |
| [Pi backend](pi_backend.md) | Agent isolation, structured results, and tool restrictions when using Pi. |
| [Benchmark and evaluation](benchmark.md) | How we measure the act: test cases, separation of expected answers from review inputs, and recall scoring.|
| [Related work](related_work.md) | How the skill relates to prior art (Sashiko; a growing survey). |

## Design principles

Four commitments recur throughout this specification:

1. **Recall first, precision next — measured, not asserted.**
   The benchmark measures whether ACR catches known real defects.
   Verification retains uncertain findings,
   while precision remains a future benchmark axis.
   Expected answers stay outside review inputs.
2. **Deterministic code controls the workflow.**
   Target resolution, persona activation, result validation, assembly,
   state persistence, scoring, and report writing follow explicit rules.
   These mechanics remain reproducible across model providers
   and cannot silently discard findings.
3. **The coding guidelines are shared standard.**
   Guideline-backed findings cite the fetched rule by its short name.
   Concrete defects without a matching rule remain reportable
   when grounded in an explicit explanation.
4. **Narrow agent capabilities.**
   Persona review, verification, consolidation, and summary each receive
   a bounded task, permitted **read-only tools**, and a validated output contract.
   Agents do not edit reviewed code or write the report.
   Keeping orchestration in code confines model work to judgment and reduces time and token use.
