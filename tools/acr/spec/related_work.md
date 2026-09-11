# Related Work

How ACR relates to existing automated code-review systems.
This page starts with the closest prior art and will grow as the survey expands.

## Sashiko

Sashiko is an AI-assisted review system for Linux and the closest prior art.
ACR differs in two deliberate ways:

- **Local, not a service.**
  Sashiko runs as a web service.
  ACR runs against a local checkout and writes a file,
  so the same tool serves a maintainer at a terminal and an agent inside a write → test → review loop.
- **Subjective as well as objective findings.**
  Sashiko reports only undeniable bugs,
  because subjective comments have no agreed standard.
  Asterinas has that standard, its Coding Guidelines,
  so ACR also reports maintainability and documentation issues,
  each grounded in a rule it has fetched.
  Concrete bugs that no rule names are still reported
  — each grounded in a short plain-language description of the defect, not a bare `bug`.

"Grounding subjective findings in rules" is what makes the persona design necessary
(see [Coding guidelines](coding_guidelines.md)).
