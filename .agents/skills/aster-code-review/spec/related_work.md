# Related Work

How `aster-code-review` relates to existing automated code-review systems.
AI-assisted review is moving quickly;
this page starts with the closest prior art and will grow as the survey expands.

## Sashiko

Sashiko is an AI-powered review system for Linux and the closest prior art.
`aster-code-review` differs deliberately on two axes:

- **Local-first, not a service.**
  Sashiko runs as a web service;
  `aster-code-review` runs locally against the repository,
  with no server — which is what lets the *same* skill serve a human at a terminal and an agent inside a loop.
- **Subjective *and* objective.**
  Sashiko restricts itself to undeniable bugs,
  because subjective calls have no agreed standard to anchor them.
  Asterinas *has* that standard
  — the Coding Guidelines — so `aster-code-review` makes subjective calls too,
  **each grounded in a cited rule** (see the comment model in [`interface.md`](interface.md)).
  The objective half survives as a first-class category for defects no rule names
  — each grounded in a short plain-language description of the defect, not a bare `bug`.
  Grounding is what turns "this is bad" from an opinion into a reviewable,
  standard-backed judgement; it is the hinge the whole persona design turns on (see [`coding_guidelines.md`](coding_guidelines.md)).
