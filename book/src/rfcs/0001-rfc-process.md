# RFC-0001: RFC process

* Status: Draft
* Pull request: https://github.com/asterinas/asterinas/pull/2365/
* Date submitted: 2025-08-24
* Date approved: YYYY-MM-DD

## Summary

The "RFC" (request for comments) process is intended to provide a consistent, transparent, structured path for the community to make "big" decisions. For example, the RFC process can be used to evolve the project roadmap and the system architecture.

## Motivation

As the Asterinas project grows in scale and community size, an informal decision-making process becomes insufficient. A formalized process is crucial for maintaining the quality and coherence of the project. Several highly successful open-source projects have demonstrated the value of such a process, including Rust ([RFCs](https://rust-lang.github.io/rfcs/introduction.html)), Fuchsia ([RFCs](https://fuchsia.dev/fuchsia-src/contribute/governance/rfcs)), Python ([Python Enhancement Proposals](https://peps.python.org/) or PEPs), and Kubernetes ([Kubernetes Enhancement Proposals](https://www.kubernetes.dev/resources/keps/) or KEPs).

Adopting a formal RFC process will:
- **Enhance decision quality**: By requiring a detailed proposal and public discussion, we can ensure that decisions are well-reasoned, technically sound, and consider a wide range of perspectives.
- **Foster Consensus**: The process provides a clear framework for the community to come together, discuss differing viewpoints, and arrive at a consensus on important issues.
- **Create design records**: Approved RFCs will serve as a historical archive, documenting the design choices made and the reasoning behind them. This is invaluable for new contributors and for future reference.

By introducing this process, we aim to support the healthy and sustainable growth of the Asterinas project.

## Design

### When to follow the RFC process

The RFC process should be used for any "substantial" change to Asterinas. Examples of changes that **require** an RFC include:
- Launching new top-level sub-projects of a similar significance to OSTD and OSDK.
- Defining or significantly altering the project roadmap and goals.
- Establishing or changing project-wide norms, such as this RFC process itself or coding style guides.
- Proposing significant architectural designs, such as the framekernel architecture or safe policy injection.
- Introducing changes that affect user-space programs, such as adding a non-Linux system call or feature.

Examples of changes that **do not** require an RFC include:
- Proposing a design whose impact is confined to a single sub-project or module. If the design is significant or non-trivial, submit a design proposal on Github Issues for discussion.
- Adding well-understood features with established patterns, such as a standard Linux system call or a device driver.
- Refactoring existing code, fixing bugs, or improving performance.

When in doubt, it is best to consult with the project maintainers using the methods described in the next section.

### How the RFC process works

The RFC process consists of several distinct stages:

#### The Socialization Stage

Before investing the time to write a full RFC, it is highly recommended to socialize the core idea with the community. This helps gauge interest, gather early feedback, and refine the proposal. Good venues for this include:

- Starting a discussion on the project's [GitHub Discussions page](https://github.com/asterinas/asterinas/discussions).
- Posting a "Pre-RFC" document to solicit more detailed feedback.
- Talking to key contributors, code owners, or maintainers directly.

Having support from at least a few other community members is a strong signal that the idea is ready for a formal RFC. Additionally, having a proof-of-concept implementation or identifying individuals committed to the implementation can strengthen the proposal.

#### The Draft Stage

Once you are ready to proceed, create a formal RFC document.
1. Fork the `asterinas/asterinas` repository.
2. In [the `rfcs` directory](https://github.com/asterinas/asterinas/tree/main/book/src/rfcs), copy `rfc-template.md` and rename it to `0000-your-rfc-title.md`. The `0000` is a placeholder for the RFC number, which will be assigned later.
3. Fill out the template with your proposal. The template provides a solid structure, but feel free to adapt it to best suit your proposal.

#### The Iteration Stage

Submit your RFC draft as a pull request to the Asterinas repository. Once the PR is opened, the formal review process begins:
- A project maintainer will be assigned as the **facilitator** for the RFC. The facilitator's role is to guide the discussion, ensure it remains productive, and ultimately determine when consensus has been reached.
- All members of the community are encouraged to review the proposal and provide constructive feedback through comments on the pull request.
- The RFC author is responsible for engaging with the feedback, addressing concerns, and updating the RFC text to reflect the evolving consensus.

#### The Voting Stage

When the discussion has converged and the major points of feedback have been addressed, the author can request that the facilitator initiate a final vote.

- The vote is open to all project maintainers and code owners of top-level sub-projects. See the [`CODEOWNERS`](https://github.com/asterinas/asterinas/blob/main/CODEOWNERS) file for details.
- Voters can express either "approval" or "rejection."
- The final decision will be based on a [rough consensus](https://en.wikipedia.org/wiki/Rough_consensus) as determined by the facilitator. This means that while unanimous agreement is not required, any major objections must be addressed.

If the final decision is approval:
1. The facilitator will assign an unique RFC number.
2. The author will update the RFC's file name with the assigned RFC number. The author should also update the metadata fields of the RFC.
3. The facilitator will merge the pull request.

#### The Maintenance Stage

Once an RFC is approved, the proposed changes are ready for implementation. When the corresponding work is completed and merged, the RFC's status is updated from "approved" to "implemented."

Approved RFCs are considered immutable historical records of design decisions. As such, only minor corrective edits like fixing typos or broken links are allowed.

Significant changes or amendments to an existing RFC must be proposed through an entirely new RFC. This new proposal can also be used to formally deprecate a previous RFC, in which case the original's status will be updated to "deprecated."

## License

All RFCs submitted to the Asterinas project are licensed under [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0). This license is more permissive than [the MPL license](https://www.mozilla.org/en-US/MPL/) used for the Asterinas code.

## Drawbacks

Introducing a formal RFC process can increase the overhead for making changes to the project. The process is intentionally deliberate, which may slow down the pace of development for major features. There is also a risk that the process could become overly bureaucratic if not managed carefully. For contributors, the effort required to write a high-quality RFC and see it through the review process can be significant.

## Prior Art and References

The Asterinas RFC process is heavily inspired by the well-established processes of the following two open-source projects:

- [Rust RFC Process](https://rust-lang.github.io/rfcs/0002-rfc-process.html): The overall structure and philosophy are closely modeled on Rust's RFC process.
- [Fuchsia RFC Process](https://fuchsia.dev/fuchsia-src/contribute/governance/rfcs/rfc_process): We have drawn on the Fuchsia process for its clear definition of roles and its emphasis on transparent decision-making.
