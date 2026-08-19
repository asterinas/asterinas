# Suggested Asterinas Issue

- **Title:** `Add an AppArmor-compatible LSM subset to Asterinas`
- **Initial type:** design proposal, not a release commitment
- **Template:** Feature Request, or Blank Issue when pasting the prepared body
- **Suggested label:** `C-design-proposal` (after maintainer/bot triage)

If the Feature Request template adds `C-feature-request`, comment:

```text
@boterinas label -C-feature-request C-design-proposal
```

Ask maintainers whether the cross-subsystem scope should be promoted to the
formal RFC process; do not put `[RFC]` in the initial title preemptively.

---

## Summary

This issue proposes adding an AppArmor-compatible Linux Security Module (LSM)
subset to Asterinas as a native, safe-Rust implementation.

The proposal is not to translate Linux's `security/apparmor` implementation or
to claim full AppArmor compatibility. The goal is to reproduce a deliberately
scoped set of observable AppArmor semantics on top of Asterinas's own task,
credential, VFS, exec, and component models, while exposing only the official
userspace ABI features that Asterinas implements completely.

Before implementation, this issue seeks consensus on:

1. the initial compatibility and threat-model boundary;
2. the seam between generic LSM infrastructure in `aster-core` and an
   AppArmor high-level component;
3. the staged implementation and validation plan;
4. the maintainers/code owners who can review the cross-subsystem work.

## Motivation

AppArmor is a task-centered mandatory access-control system. Profiles attach
security labels to tasks, mediate operations such as exec, file access,
capability use, and ptrace, and are normally compiled by `apparmor_parser` and
loaded through the kernel AppArmor filesystem interface.

Supporting a well-defined AppArmor subset would provide Asterinas with:

- a Linux-compatible application-confinement mechanism;
- compatibility with a pinned subset of the existing AppArmor policy toolchain;
- an end-to-end user of the Asterinas LSM framework beyond capability and Yama;
- a reusable test bed for task, exec, VFS, policy-update, and audit mediation.

Unlike proposals that require compatibility with Linux-internal C structures
or helper interfaces, this proposal targets a pinned userspace policy ABI and
observable syscall/security behavior. The implementation remains free to use
Asterinas-native Rust types and component interfaces internally.

A concrete first pilot workload must be named before implementation is placed
on a release roadmap. The issue author should state whether that workload is an
Asterinas NixOS service, an OCI/container-runtime AppArmor profile, or another
reproducible application, and link the command/test that currently has to run
unconfined.

The immediate goal is an experimental, x86_64-only system pilot. This proposal
does not claim that Asterinas or Asterinas NixOS is ready for production use.

## Current State

The current LSM framework is intentionally small:

- [`LsmModule`](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm/mod.rs#L31-L48)
  composes only capability and alien-access hook traits.
- [The current hook set](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm/hooks/mod.rs#L14-L28)
  contains only those two hook families.
- [Module registration](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm/modules/mod.rs#L31-L42)
  is a private, static list containing capability and Yama.

Asterinas already has useful implementation seams: per-thread credentials, a
central exec prepare/commit path, `Path = Mount + Dentry`, common VFS operation
paths, file handles, mmap backing files, boot parameters, and QEMU regression
tests. It does not yet have AppArmor task labels, exec/file hooks, a policy blob
decoder, AppArmor securityfs nodes, or an AppArmor audit transport.

The [current kernel organization guidelines](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/book/src/to-contribute/coding-guidelines/for-maintainability/rust-specific/crates-and-modules.md)
say that new kernel subsystems should be separate crates outside `aster-core`
by default. Therefore, the proposed placement is:

- `aster-core`: generic LSM interfaces, hook contexts, registration, and the
  minimal opaque task/file security state required by registered LSMs;
- `kernel/comps/apparmor`: AppArmor labels, profiles, rule matching, decisions,
  policy store, ABI decoder, and AppArmor-specific hook implementations;
- `distro/` and `test/`: pinned parser packaging, policy cache, early loader,
  audit consumer, and end-to-end regression tests.

This split requires a small component-registration seam because the existing
`LsmModule` and static module list are private to `aster-core`. It should not
trigger unrelated migration of capability or Yama in the same change.

## Design Principles

1. **Native safe Rust.** Code under `kernel/` remains safe Rust. The design may
   use Linux's documented ABI and observable behavior as references, but must
   not copy or mechanically translate GPL-2.0-only implementation code.
2. **Official ABI subset.** `features/` advertises only behavior implemented and
   validated end to end. If a feature bit is coarser than the implementation,
   either implement the whole feature group, select a lower ABI, or do not
   advertise it.
3. **No policy text parser in the kernel.** A pinned, unmodified
   `apparmor_parser` compiles source policy. The kernel accepts only a pinned
   binary policy ABI and validates every untrusted field before installation.
4. **One decision path.** Typed hook contexts feed one AppArmor decision engine;
   enforcement mode and structured audit are applied after the raw rule result.
5. **Atomic lifecycle changes.** Exec label transitions and policy replacement
   commit atomically. Rejected operations must not leave VFS or policy side
   effects behind.
6. **Incremental review.** Each phase is split into focused issues/PRs with
   user-visible tests. This issue is not a proposal for one large implementation
   PR.

## Initial Compatibility Target

The first installable subset should include:

- `lsm=apparmor` selection and an unconfined default;
- a root policy namespace and single-profile task labels;
- label inheritance across fork/clone and attachment/transition on exec;
- enforce and complain modes with structured, loss-detectable audit records;
- capability mediation after ordinary capability checks;
- exec attachment, `ix`, and non-fallback `px`;
- file permissions for the advertised ABI, including owner/non-owner entries,
  hard-link pair/subset semantics, mutation pre-checks, inherited/received file
  descriptors, and mmap/mprotect;
- the minimum bilateral ptrace checks required to stop a tracer from retaining
  control across a protected exec or profile replacement;
- official `features`, `profiles`, load/replace/remove, and current-label
  interfaces for one pinned parser/policy ABI;
- build-time policy cache generation and early initramfs loading.

The exact feature manifest and parser version are outputs of the compatibility
spike below, not assumptions of this proposal.

## Proposed Roadmap

Each checkbox should become one or more dedicated issues and focused PRs after
the relevant design is accepted.

- [ ] **Freeze the compatibility contract and threat model**
  - Pin an Asterinas commit, Linux AppArmor reference commit, parser tag, policy
    ABI, and feature ABI.
  - Generate a golden policy-blob corpus for every advertised permission,
    transition, qualifier, and invalid combination.
  - Establish a pinned Linux differential oracle for syscall result/errno,
    labels, audit classification, policy replacement, and open-resource
    behavior.
  - Confirm the implementation/licensing boundary with maintainers.

- [ ] **Add a bootable unconfined AppArmor component skeleton**
  - Add the minimum high-level-component registration seam to the LSM core.
  - Boot with `lsm=apparmor` while leaving all tasks unconfined and preserving
    baseline behavior.
  - Add explicit initialization and task-label inheritance tests.

- [ ] **Complete the first capability vertical slice**
  - Implement label -> profile -> rule -> decision -> mode -> audit -> syscall
    result.
  - Audit and route all security-relevant capability checks through the LSM
    hook before advertising the capability feature.
  - Restore the missing `CAP_SYS_ADMIN` baseline on legacy
    mount/umount/pivot-root paths before relying on path confinement.

- [ ] **Add subject-visible paths and exec domains**
  - Define path semantics for mount namespaces, chroot, rename races, deleted
    paths, and `O_TMPFILE`.
  - Add exec prepare/commit hooks, attachment, `ix`, and non-fallback `px`.
  - Cover scripts/interpreters, set-id/file capabilities, `no_new_privs`, and
    existing ptrace relationships.

- [ ] **Close the file-mediation lifecycle**
  - Place create/truncate/rename/link/unlink and metadata hooks before side
    effects.
  - Cover open/read/write/append/lock, inherited descriptors, SCM_RIGHTS,
    mmap/mprotect, owner qualifiers, and hard-link subset checks.
  - Do not advertise the file feature while an operation or qualifier in the
    selected ABI can bypass mediation.

- [ ] **Add the official policy control plane**
  - Implement the minimum securityfs/AppArmor nodes used by the pinned parser.
  - Strictly decode and atomically install the pinned policy blob format.
  - Add profiles/current-label introspection, a loss-detectable audit reader,
    and the minimum ptrace lifecycle checks.
  - Reject unsupported source classes in the fixed parser pipeline and reject
    unknown/invalid blobs in the kernel.

- [ ] **Package and validate an experimental system pilot**
  - Build policy caches with the pinned parser instead of compiling policy in
    the initramfs.
  - Give the initial loader a one-shot bootstrap authority, load and verify all
    required profiles, then freeze policy updates before starting other user
    tasks.
  - Start protected services through a confined launcher and non-fallback `px`
    so the target image never runs unconfined after an attachment mismatch.
  - Provide a last-known-good boot entry, audit consumer, health checks, and a
    tested maintenance-reboot rollback path.

## Security Invariants

The initial pilot must not be declared successful unless all of these hold:

- an AppArmor allow never overrides an ordinary DAC/capability denial;
- malformed or unsupported policy is rejected without partially replacing the
  active snapshot;
- denied VFS mutations produce no object, data, metadata, or topology change;
- legacy mount operations cannot bypass path policy through a missing ordinary
  capability check, and protected pilot services do not receive
  `CAP_SYS_ADMIN` before AppArmor mount mediation exists;
- attach-before-exec, `PTRACE_TRACEME`, and post-replacement ptrace commands
  cannot retain unauthorized control of a confined task;
- a missing required profile or wrong launcher/target label fails before the
  protected target image executes;
- advertised file behavior includes the same-ABI owner/non-owner and hard-link
  conditions rather than treating them as unconditional allows;
- audit loss is detectable through sequence/lost counters; the design does not
  claim that a finite ring is lossless.

## Validation

Every implementation phase should include the smallest relevant subset of:

```bash
make check
make test
make ktest
make run_kernel AUTO_TEST=regression
```

Validation should test user-visible behavior rather than internal constants.
For each advertised feature, include at least an allow case, a missing-allow
denial, an explicit-deny case, and malformed/unsupported policy input. The same
policy and workload should also be compared with the pinned Linux AppArmor
oracle where compatibility is claimed.

The first runtime target is x86_64. RISC-V and LoongArch should not be included
in the support statement until their security regression suites run in CI.

## Documentation and CI Plan

The implementation should update the Asterinas Book with:

- the supported/unsupported AppArmor feature and policy ABI matrix;
- boot parameters and the exact parser/cache build contract;
- policy-management permissions, bootstrap/freeze behavior, and audit fields;
- experimental deployment, health checking, maintenance update, and rollback;
- explicit differences from the pinned Linux AppArmor oracle.

CI should keep the unconfined baseline, policy-input negative corpus, advertised
feature regressions, and x86_64 boot/deployment test separate enough to identify
which contract failed. Other architectures remain compile-only until their
runtime security tests are added.

## Drawbacks and Alternatives

This proposal adds a long-lived compatibility and security-review burden. LSM
hooks affect hot and security-sensitive paths, the policy decoder processes
hostile input, and compatibility must be rechecked when the pinned AppArmor ABI
is upgraded. These costs are justified only if a concrete pilot workload and
maintainer/reviewer ownership are agreed.

Alternatives considered:

- **Translate Linux `security/apparmor` to Rust.** This does not fit Asterinas's
  object model or safe-Rust/component rules and creates a serious licensing and
  maintenance problem.
- **Add a custom AppArmor-like policy language.** This can demonstrate a deny
  quickly but does not interoperate with the official parser or profile
  ecosystem. It may be used only as a test fixture before the official decoder
  exists, never as a user-facing ABI.
- **Add generic LSM hooks without an AppArmor consumer.** Some hook work is
  necessary, but speculative hook families and blob registries should not land
  ahead of a concrete consumer and user-visible test.

## Non-goals for the Initial Subset

- full compatibility with every distribution profile under `/etc/apparmor.d`;
- Linux AppArmor implementation-internal compatibility;
- a custom AppArmor-like policy language or kernel text parser;
- compound/stacked labels, hierarchical policy namespaces, hats, and arbitrary
  `change_profile`/onexec behavior;
- AppArmor signal, rlimit, mount-rule, network, Unix-peer, DBus, mqueue,
  io_uring, prompt, or kill mediation;
- Linux audit-netlink wire compatibility; the first pilot uses an explicitly
  Asterinas-specific, loss-detectable audit transport;
- live policy updates in the first system pilot;
- production-readiness claims for Asterinas NixOS.

These features should be proposed separately only after the initial hooks,
object lifecycles, and user demand exist.

## Open Questions for Maintainers

1. Is a generic LSM seam in `aster-core` plus an AppArmor high-level component
   the right placement under the current kernel crate guidelines?
2. What is the smallest acceptable core-owned representation for component
   task/file security state and clone/exec lifecycle callbacks?
3. Is the proposed initial compatibility boundary small enough to review while
   still honest with respect to AppArmor feature-ABI granularity?
4. Which maintainers/code owners can review the generic LSM, exec/VFS, and
   userspace-policy parts?
5. Should this proposal remain unassigned to a release until lead developers
   and reviewers explicitly agree on ownership?
6. What exact service or container workload is the first deployment pilot, and
   what observable confinement test defines success for it?

## Coordination

After architectural consensus, this issue can become the umbrella tracking
issue. Each roadmap item should link to a dedicated child issue/PR. Preparatory
refactoring should land separately from feature behavior, and each PR should
represent one logical change with its own regression coverage.

No release target is proposed yet. Adding this work to a release plan should be
a separate social commitment between named implementers and reviewers.

Before posting this proposal, the author should add one short paragraph stating
their project context and concrete commitment, for example which of the
compatibility spike, LSM skeleton, or later phases they intend to implement.

## References

- [Linux kernel AppArmor documentation](https://docs.kernel.org/admin-guide/LSM/apparmor.html)
- [Linux AppArmor LSM hooks](https://github.com/torvalds/linux/tree/master/security/apparmor)
- [AppArmor parser and userspace project](https://gitlab.com/apparmor/apparmor)
- [Asterinas kernel component proposal, #3601](https://github.com/asterinas/asterinas/issues/3601)
- [Asterinas RISC-V IOMMU tracking-issue structure, #3474](https://github.com/asterinas/asterinas/issues/3474)
- [Asterinas seccomp design proposal, #3648](https://github.com/asterinas/asterinas/issues/3648)
- [Asterinas issue templates](https://github.com/asterinas/asterinas/tree/main/.github/ISSUE_TEMPLATE)
- [Asterinas contribution guidelines](https://github.com/asterinas/asterinas/tree/main/book/src/to-contribute/coding-guidelines)
