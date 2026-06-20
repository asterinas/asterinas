# Coding Guidelines

This section describes coding and collaboration conventions for the Asterinas project.
These guidelines aim to keep code clear, consistent, maintainable, correct, and efficient.

These guidelines set the same standard for both **writing** code and **reviewing** it.
They are organized by **persona**:
five durable engineering roles,
each a page that doubles as that persona's checklist.

See [How Guidelines Are Written](how-guidelines-are-written.md)
for the philosophy and the bar every guideline must meet (concrete, concise, grounded, relevant).

## The Five Personas

In a kernel-development team,
engineers largely divide into five roles, or *personas*,
each with its own area of expertise and its own focus:

| Persona | Focus | Guidelines |
|---|---|---|
| Project maintainer | Is the code well-shaped and understandable? | [For Maintainability](for-maintainability/) |
| Kernel developer | Is it correct and efficient? | [For Development](for-development/) |
| Security expert | Is it safe and secure? | [For Security](for-security/) |
| Hardware expert | Is it correct against the hardware contract? | [For Hardware](for-hardware/) |
| Documentation writer | Are the user-facing docs well-written? | [For Documentation](for-documentation/) |

Each persona page opens with an index (the third column of the table above) for its guidelines
so a reader (or a review tool) can grasp the gist of a guideline,
reading the full text only when needed.

## Where a Guideline Belongs {#where-a-guideline-belongs}

A guideline belongs to the persona that is the **natural owner of the failure it prevents** —
the one who guards against it when writing
and catches it when reviewing,
and whose context holds the evidence to judge a violation.

Some failure modes look interesting to two personas at once.
The boundaries that come up most, and how to draw them:

- **Project maintainer vs. kernel developer.**
  Ask whether the concern is a matter of *judgment* or of *objective correctness*.
  Shape, naming, and design are judgment calls the **maintainer** owns;
  a logic error or a broken invariant is an objective defect any **developer** can point to undeniably.
- **Kernel developer vs. security expert.**
  A logic or concurrency bug is the **developer**'s when it is merely *wrong*,
  and the **security expert**'s when an adversary can *exploit* it.
  Any `unsafe`-soundness or untrusted-input guidelines always belong to the security expert.
- **Kernel developer vs. documentation writer.**
  Ask whether the failure is in the *code* or in the *documentation about the code*.
  A user-facing page or coverage file (e.g., `README.md`) that no longer matches the code —
  drifted, missing, or misleading —
  is the **documentation writer**'s.
- **Project maintainer vs. documentation writer.**
  In-code comments and rustdoc are written *for the next maintainer or developer*,
  so they belong to the **maintainer**;
  the **documentation writer** owns standalone, user-facing artifacts such as the book and the syscall coverage files.
  Same medium — prose — but a different reader whose failure it is.

## Adding a Guideline {#adding-a-guideline}

- Find the persona that owns the failure your guideline prevents (the [test above](#where-a-guideline-belongs)),
  and add it to the matching subsection of that persona's page,
  with an entry in the page's Index.
- **Language- and path-specific guidelines are not new top-level categories** —
  they live inside the owning persona.
  Rust-specific guidance goes in the persona's **Rust-Specific** subsection, grouped by language item,
  and each persona reserves a **Path-Specific** slot for guidelines scoped to one repository path —
  a sub-project (such as `ostd`), an architecture directory, or a kernel subsystem.
  For example, the Linux-compatibility doc guideline lives under For Documentation → Path-Specific → `kernel/`.
- Keep a guideline's kebab-case **short-name** stable once published;
  links point at it.
