# Persona-Keyed Coding Guidelines

ACR consumes Asterinas's
[Coding Guidelines](../../../../../book/src/to-contribute/coding-guidelines/).
The book is the source of truth; ACR's
[persona prompts](../prompts/personas/) describe review responsibilities without
copying the full rule corpus.

## Why guidelines are organized by persona

Writing and reviewing are two views of the same rule. A page that belongs to a
stable reviewer role can guide an author and also serve as that reviewer's
checklist. Topic-only groupings such as “Rust” or “testing” do not provide that
ownership: one topic spans several kinds of failure, while one reviewer remit
spans several topics.

The persona organization provides:

- **stability**: reviewer roles change less often than languages, subsystems,
  or tools;
- **ownership**: every rule has a natural reviewer responsible for the failure
  it prevents;
- **selective exposure**: one review pass receives only its own persona block;
- **progressive disclosure**: an index supplies short-name and gist first, and
  exact rule text is fetched only for a concrete suspicion.

## The five personas

| Persona | Review question | Template |
|---|---|---|
| Maintainability | Is the change well-shaped and understandable over time? | [`maintainability.md`](../prompts/personas/maintainability.md) |
| Development | Is it correct and efficient across inputs, failures, and schedules? | [`development.md`](../prompts/personas/development.md) |
| Security | Can hostile input or unsafe behavior violate kernel security or soundness? | [`security.md`](../prompts/personas/security.md) |
| Hardware | Does low-level code obey the architecture, device, and ABI contract? | [`hardware.md`](../prompts/personas/hardware.md) |
| Documentation | Are user-facing documents and compatibility artifacts correct and current? | [`documentation.md`](../prompts/personas/documentation.md) |

Testing belongs to Development because proving and preserving correctness is a
developer responsibility. Security and Hardware are separate from general
correctness because they require adversarial and platform-specific context.

## Placement principle

A rule belongs to the persona that naturally owns the failure it prevents and
has the evidence needed to judge that failure. The subject matter alone is not
enough. For example, a module-documentation rule may belong to Maintainability
when its purpose is to let future maintainers navigate the code; a boundary
validation rule belongs to Security when its purpose is to stop untrusted input
crossing a trust boundary.

This rule keeps persona scopes bounded and reduces duplicate investigation.

## Progressive disclosure

Prompt construction in [`stages/prompts.py`](../stages/prompts.py) builds a
complete short-name/gist catalog for each activated persona. The catalog
includes a digest that pins the rule corpus used for the pass. A reviewer with
a concrete suspected violation calls the read-only `guideline.show` tool with:

- the owning persona;
- the catalog digest;
- one or more candidate short-names.

[`core/disclosure.py`](../core/disclosure.py) and
[`scripts/print_guideline.py`](../scripts/print_guideline.py) validate the
digest and return the exact authored rule sections. A finding may cite a
short-name only after this lookup. This keeps prompts smaller without weakening
traceability.

Both model backends consume the same `guideline.show` ToolBroker contract. The
Pi adapter registers a model-facing `guideline_show` definition, proxies the
call back to Python, and ultimately invokes the same colocated
`print_guideline.py`; Pi does not receive a filesystem path that lets it bypass
the digest check. The script path and SHA-256 are recorded as host metadata,
not injected as untrusted prompt text.

`ACR_GUIDELINE_DISCLOSURE=progressive` is the normal mode. `full` is retained as
an internal comparison and rollback setting; it does not change the report
contract.

## Guideline source selection

The supported [`run.sh`](../run.sh) entry point selects a trusted guideline root
before starting Python:

1. use an explicit `ACR_GUIDELINE_ROOT` when supplied;
2. otherwise use a bundled `guideline-root/` snapshot if present;
3. otherwise use the Git checkout containing `run.sh`;
4. fail if none contains the guideline corpus.

This lets ACR review a historical or detached target checkout with the current
review rules. The benchmark relies on the same separation: target code comes
from a detached worktree, while guideline data comes from the controller's
trusted repository. See [Benchmark and evaluation](benchmark.md#answer-key-and-resource-isolation).
