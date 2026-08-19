# AppArmor M1 Capability Confinement Design

## Status

Approved in chat on 2026-08-19.

## Goal

Deliver the first runnable AppArmor vertical slice for Asterinas. A process can acquire a named profile after a successful `exec`, retain that profile across `fork` and `clone`, and be denied a Linux capability even when its ordinary credential capability set would otherwise allow the operation.

M1 also provides minimum ptrace isolation and an auditable denial record. It does not implement policy loading or resource rules outside capabilities and ptrace.

## Scope

M1 includes:

- An `aster-apparmor` high-level kernel component.
- Immutable profiles with enforce and complain modes.
- Per-POSIX-thread AppArmor labels.
- Exact executable-path profile attachment after successful `exec`.
- Label inheritance for process and thread creation.
- Capability allow-list enforcement through the existing LSM capability hook.
- Same-profile ptrace by default and explicit permission for cross-profile ptrace.
- Structured kernel log records for denied operations.
- Unit tests for policy decisions and an initramfs regression test for the runnable path.

M1 excludes:

- Userspace policy syntax and parsing.
- Runtime policy replacement.
- `securityfs` or another policy control filesystem.
- File, mount, network, signal, IPC, namespace, or resource-limit rules.
- Linux AppArmor ABI compatibility.
- A general audit subsystem.

## Naming and Placement

The Cargo package is named `aster-apparmor`, imported by Rust code as `aster_apparmor`, and stored in `kernel/core/comps/apparmor`, matching the existing workspace component path. The LSM module remains named `apparmor`.

This follows the existing Asterinas convention of an `aster-` package prefix and a functional directory/module name.

## Component Model

`aster-apparmor` owns policy data and pure policy decisions. It must not depend on `aster-core`, because `aster-core` consumes the component.

The component exposes these concepts:

- `Profile`: immutable name, mode, and capability rules.
- `ProfileMode`: `Enforce` or `Complain`.
- `CapabilityRules`: a Linux capability bitmask interpreted as an allow-list.
- `TaskLabel`: a non-optional reference to a profile.
- Profile lookup by exact resolved executable path.

Capability identifiers cross the component boundary as their stable Linux numeric value. Conversion from Asterinas's internal `Capability` type remains in the core LSM module, avoiding a dependency cycle.

Profiles are compile-time static in M1. The table contains:

- `kernel/default`: the initial named profile, allowing all currently supported capabilities so existing boot behavior remains unchanged for tasks that keep this profile.
- `apparmor/m1-capability-test`: selected only for the initramfs test executable at `/test/security/lsm/apparmor`, denying the capability exercised by that test.

The static table is the M1 policy source, not a public runtime configuration interface. A later policy-loading milestone may replace its lookup implementation without changing task labels or LSM enforcement.

## Label State and Concurrency

Every `PosixThread` has exactly one `TaskLabel`; there is no `None` value and no implicit unconfined state. The label is stored behind the existing kernel synchronization primitives and refers to an immutable static profile.

`PosixThreadBuilder` initializes new threads with `kernel/default`. This guarantees that a fully constructed task is always labeled, including the initial task.

The AppArmor task-clone hook copies the parent's label to the child before the child is inserted into a process task set, PID table, or scheduler-visible state. Both thread-clone and process-clone paths invoke the hook.

## Exec Transition

The executable identity used for matching is the resolved absolute path of the originally requested executable. Interpreter loading must not replace a script's executable identity with the interpreter path.

An exec transition follows this sequence:

1. Resolve the executable and load the new program as the existing exec path does today.
2. Preserve the resolved executable identity while the old process image is still recoverable.
3. Perform the existing irreversible exec commit.
4. Commit the prepared AppArmor label only after the new process image has been installed successfully.

If no profile table entry matches, the task keeps its current label. This is inheritance, not fallback to an unconfined label. A confined task therefore cannot escape confinement by executing an unmatched binary.

The core prepares the executable identity before the irreversible boundary, then invokes an infallible LSM task-exec commit hook after the new image is installed. A failed exec therefore cannot leave the old process image carrying the new executable's profile.

## Capability Enforcement

The existing capability LSM remains responsible for credential and user-namespace capability semantics. AppArmor is an additional restriction and never grants a capability.

For a capability request:

1. Existing capability checks must allow the request.
2. The current AppArmor profile's capability allow-list must also allow it.
3. If AppArmor denies it in enforce mode, return `EPERM`.
4. If AppArmor denies it in complain mode, emit the denial record and allow the request.

The result is an intersection: ordinary credentials AND the AppArmor profile must authorize the operation.

## Ptrace Enforcement

M1 adds a minimum AppArmor check to the existing ptrace LSM path:

- Tracing is allowed by AppArmor when tracer and tracee have the same profile.
- Tracing a task with a different profile is always denied in M1; there is no configurable ptrace rule.
- Enforce mode returns `EPERM` for a cross-profile request.
- Complain mode logs the denial and allows the existing ptrace checks to continue.

Existing Yama and capability checks still apply. AppArmor cannot override their denials.

## Audit Record

M1 uses the existing kernel logger instead of introducing an audit framework. Each AppArmor denial emits one structured warning containing at least:

- `apparmor="DENIED"`
- operation (`capable` or `ptrace`)
- profile name
- task identifier
- requested capability number or target task identifier
- profile mode

The log is emitted in both enforce and complain modes. The final result distinguishes whether the denial was enforced or reported only.

## LSM Registration

The core AppArmor LSM module implements the existing capability and ptrace hook families and the task lifecycle hooks needed by clone and exec.

`apparmor` is added to the known LSM module list and the default optional module list. The named permissive default profile ensures that enabling the module does not alter unrelated boot behavior. Existing `lsm=` module-selection behavior remains authoritative.

## Error Handling

- Capability and ptrace policy denials use `EPERM`.
- Executable identity preparation completes before the existing irreversible exec boundary.
- Exec commit does not allocate and cannot fail.
- Clone hook failure, if any future implementation introduces one, must occur before publishing or running the child.
- Missing exact-path profile entries inherit the current profile.
- Invalid capability numbers are denied by the pure policy layer rather than indexing outside the bitmask.

## Test Strategy

Implementation follows red-green-refactor.

Component unit tests cover:

- Allowed and denied capability bits.
- Out-of-range capability identifiers.
- Enforce versus complain decisions.
- Fixed same-profile allow and cross-profile deny ptrace decisions.
- Exact-path matching and unmatched-path inheritance.

The initramfs regression test at `/test/security/lsm/apparmor` covers:

- Profile attachment after exec.
- A capability-gated operation denied with `EPERM` under the restrictive profile.
- The same denial after fork, proving inheritance.
- A capability retained by the profile reaching the underlying syscall instead of being rejected by AppArmor.

The existing LSM module-selection regression test is extended only as needed to recognize `apparmor` as a selectable optional module.

## Acceptance Criteria

M1 is complete when:

- The kernel builds with the new component and AppArmor LSM enabled.
- Every POSIX task always has a named AppArmor label.
- The test executable receives `apparmor/m1-capability-test` only after successful exec.
- Forked and cloned children inherit the parent's label before they can run.
- A disallowed capability returns `EPERM` and emits a structured denial record.
- Complain mode emits the same record without enforcing the denial.
- AppArmor never grants a capability rejected by existing credential checks.
- Same-profile ptrace is not rejected by AppArmor, while cross-profile ptrace is rejected.
- Existing LSM module selection and unrelated boot behavior remain intact.
- The focused component and initramfs regression tests pass.

## Expected Files

- `kernel/core/comps/apparmor/Cargo.toml`
- `kernel/core/comps/apparmor/src/lib.rs`
- `kernel/core/src/security/lsm/hooks/mod.rs`
- `kernel/core/src/security/lsm/modules/mod.rs`
- `kernel/core/src/security/lsm/modules/apparmor.rs`
- `kernel/core/src/process/posix_thread/mod.rs`
- `kernel/core/src/process/posix_thread/builder.rs`
- `kernel/core/src/process/clone.rs`
- `kernel/core/src/process/execve.rs`
- `test/initramfs/src/regression/security/lsm/apparmor.c`
- `test/initramfs/src/regression/security/lsm/Makefile`

Workspace manifests are changed only if the existing `aster-apparmor` references are incomplete.
