# AppArmor M1 Capability Confinement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a runnable AppArmor M1 path that labels tasks on exec, inherits labels across clone, restricts Linux capabilities, and blocks cross-profile ptrace.

**Architecture:** A small `aster-apparmor` component owns immutable static profiles and pure decisions without depending on `aster-core`. `aster-core` stores a non-optional label on each POSIX thread and supplies lifecycle data to an AppArmor LSM module. The module only removes authority: existing capability, Yama, and ptrace checks remain authoritative.

**Tech Stack:** Rust `no_std`, Asterinas component crates, Asterinas LSM hooks, spin-based kernel synchronization, C initramfs regression tests.

**Spec:** `docs/superpowers/specs/2026-08-19-apparmor-m1-capability-design.md`

## Global Constraints

- Cargo package name is `aster-apparmor`; Rust import name is `aster_apparmor`; component path is `kernel/core/comps/apparmor` as already declared by the workspace.
- Do not add a policy parser, policy filesystem, runtime profile mutation, or non-capability resource rules.
- Every POSIX task has a non-optional named label; unmatched exec inherits the current label.
- AppArmor never grants a capability and returns `EPERM` for enforced denials.
- M1 ptrace behavior is fixed: same profile allowed, cross-profile denied; no configurable ptrace rule type.
- Denials log through the existing kernel logger; do not add an audit subsystem.
- The exact M1 test executable path is `/test/security/lsm/apparmor`.
- Follow existing repository formatting and SPDX conventions; add no dependency unless already present in the workspace.
- Work in the shared checkout without creating commits or changing unrelated existing edits.

---

### Task 1: Pure AppArmor policy component

**Files:**

- Create: `kernel/core/comps/apparmor/Cargo.toml`
- Create: `kernel/core/comps/apparmor/src/lib.rs`
- Modify only if the existing entries are incomplete: `Cargo.toml`
- Modify only if the existing entries are incomplete: `kernel/core/Cargo.toml`

**Interfaces:**

- Consumes: Linux capability numbers in the inclusive range `0..=63` and resolved absolute executable paths as byte slices.
- Produces: `ProfileMode`, `CapabilityRules`, `Profile`, `TaskLabel`, `Decision`, `KERNEL_DEFAULT_PROFILE`, `M1_TEST_PROFILE`, `decide_capability`, `decide_ptrace`, and `label_for_exec`.

Use these public shapes unless an existing repository lint requires an equivalent spelling:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileMode { Enforce, Complain }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision { Allow, Deny, Complain }

#[derive(Clone, Copy, Debug)]
pub struct CapabilityRules { allowed: u64 }

pub struct Profile {
    name: &'static str,
    mode: ProfileMode,
    capabilities: CapabilityRules,
}

#[derive(Clone, Copy)]
pub struct TaskLabel(&'static Profile);

pub fn decide_capability(label: TaskLabel, capability: u8) -> Decision;
pub fn decide_ptrace(tracer: TaskLabel, tracee: TaskLabel) -> Decision;
pub fn label_for_exec(current: TaskLabel, path: &[u8]) -> TaskLabel;
```

`CapabilityRules::allows` must reject values above 63 before shifting. `decide_ptrace` compares profile identity: identical profile references allow; different profiles return `Deny` or `Complain` according to the tracer profile's mode.

The static test profile denies Linux `CAP_SYS_CHROOT` (18) and allows the other representable capabilities. `label_for_exec` transitions only on the exact byte path `/test/security/lsm/apparmor`; every other path returns `current` unchanged.

- [ ] **Step 1: Write failing component tests**

Add `#[cfg(test)]` tests in `src/lib.rs` with literal expectations for:

```rust
assert_eq!(decide_capability(TaskLabel::new(&M1_TEST_PROFILE), 18), Decision::Deny);
assert_eq!(decide_capability(TaskLabel::new(&M1_TEST_PROFILE), 0), Decision::Allow);
assert_eq!(decide_capability(TaskLabel::new(&M1_TEST_PROFILE), 64), Decision::Deny);
assert_eq!(label_for_exec(TaskLabel::kernel_default(), b"/test/security/lsm/apparmor").profile().name(), "apparmor/m1-capability-test");
assert_eq!(label_for_exec(TaskLabel::new(&M1_TEST_PROFILE), b"/bin/sh").profile().name(), "apparmor/m1-capability-test");
assert_eq!(decide_ptrace(TaskLabel::new(&M1_TEST_PROFILE), TaskLabel::new(&M1_TEST_PROFILE)), Decision::Allow);
assert_eq!(decide_ptrace(TaskLabel::kernel_default(), TaskLabel::new(&M1_TEST_PROFILE)), Decision::Deny);
```

Add a local static complain profile and assert denied capability and cross-profile ptrace return `Decision::Complain`.

- [ ] **Step 2: Run the focused test and observe RED**

Run:

```bash
cargo test -p aster-apparmor --lib
```

Expected: compilation fails because the tested policy types/functions are not implemented yet, not because of a manifest syntax error.

- [ ] **Step 3: Implement the minimum pure policy engine**

Create a `#![no_std]`, `#![deny(unsafe_code)]` crate. Implement only the interfaces and two static profiles required above. Keep profile lookup as a static exact-path table or direct exact comparison; do not build a parser or registry abstraction.

- [ ] **Step 4: Run the focused test and observe GREEN**

Run:

```bash
cargo test -p aster-apparmor --lib
```

Expected: all component tests pass with no warnings introduced by this crate.

- [ ] **Step 5: Self-review the component boundary**

Confirm `aster-apparmor` does not depend on `aster-core`, no capability shift can overflow, and unmatched exec returns the incoming label rather than the default label.

---

### Task 2: Task labels and lifecycle hooks

**Files:**

- Modify: `kernel/core/src/process/posix_thread/mod.rs`
- Modify: `kernel/core/src/process/posix_thread/builder.rs`
- Modify: `kernel/core/src/security/lsm/hooks/mod.rs`
- Modify: `kernel/core/src/process/clone.rs`
- Modify: `kernel/core/src/process/execve.rs`

**Interfaces:**

- Consumes: `aster_apparmor::TaskLabel` and `TaskLabel::kernel_default()` from Task 1.
- Produces: `PosixThread::apparmor_label() -> TaskLabel`, `PosixThread::set_apparmor_label(TaskLabel)`, an infallible task-exec LSM hook carrying the resolved executable path, and active calls to the existing task-init/task-clone dispatchers.

- [ ] **Step 1: Add the non-optional label storage**

Add one synchronized `TaskLabel` field to `PosixThread`. Initialize it in `PosixThreadBuilder::build` with `TaskLabel::kernel_default()`. Expose copy-in/copy-out methods instead of exposing the lock:

```rust
pub(crate) fn apparmor_label(&self) -> TaskLabel;
pub(crate) fn set_apparmor_label(&self, label: TaskLabel);
```

Use the synchronization primitive already used by nearby mutable per-thread fields.

- [ ] **Step 2: Add the infallible exec-commit hook**

Extend `LsmTaskHook` with a default no-op callback equivalent to:

```rust
fn on_task_exec(&self, _task: &Task, _executable_path: &[u8]) {}
```

Add the matching dispatcher using the same module iteration/order as existing task hooks. Do not return `Result` from commit.

- [ ] **Step 3: Activate task initialization and clone hooks**

Invoke task initialization after `PosixThreadBuilder` has produced a complete `Task`. Invoke task clone for both `CLONE_THREAD` and new-process paths after constructing the child task but before publishing or running it. Preserve existing cleanup/error ordering.

- [ ] **Step 4: Commit exec labels only after successful image replacement**

Before the irreversible exec boundary, preserve the resolved absolute path of the originally requested executable as owned bytes. Pass it into `do_execve_no_return`. Invoke the task-exec dispatcher at the end of the successful commit path, after the new process image and credentials are installed but before returning `Ok(())`.

Do not use the ELF interpreter path for scripts. Do not change the label on any recoverable exec failure path.

- [ ] **Step 5: Compile-check core wiring**

Run:

```bash
cargo check -p aster-core
```

Expected: core compiles; existing LSM modules require no changes because the new hook has a default implementation.

- [ ] **Step 6: Self-review publication ordering**

Confirm child label inheritance hooks execute before scheduler visibility and exec label commit cannot run on an error return.

---

### Task 3: AppArmor LSM enforcement and runnable regression

**Files:**

- Create: `kernel/core/src/security/lsm/modules/apparmor.rs`
- Modify: `kernel/core/src/security/lsm/modules/mod.rs`
- Create: `test/initramfs/src/regression/security/lsm/apparmor.c`
- Modify: `test/initramfs/src/regression/security/run_test.sh`

**Interfaces:**

- Consumes: Task 1 decision functions and Task 2 label accessors/task hooks.
- Produces: the selectable/default-optional `apparmor` LSM module, capability enforcement, fixed cross-profile ptrace enforcement, clone inheritance, exec transition, and an end-to-end regression binary.

- [ ] **Step 1: Write the failing initramfs regression test**

Create `apparmor.c` using `../../common/test.h`. Add three observable behaviors:

```c
FN_TEST(profile_denies_sys_chroot)
{
    char dir[] = "/tmp/apparmor-chroot-XXXXXX";
    TEST_RES(mkdtemp(dir), _ret != NULL);
    TEST_ERRNO(chroot(dir), EPERM);
    TEST_SUCC(rmdir(dir));
}
END_TEST()
```

Add a fork test where the child repeats the `chroot` denial and exits zero, and the parent requires a normal zero exit status. Add an allowed-capability test that creates a temporary file and successfully changes its owner to UID 1, proving `CAP_CHOWN` reaches the underlying syscall.

Append `./lsm/apparmor` to `security/run_test.sh`.

- [ ] **Step 2: Build the regression binary**

Run:

```bash
make -C test/initramfs/src/regression/security/lsm
```

Expected: the new C test compiles. On a kernel without the AppArmor module, the chroot assertions are the intended behavioral RED because root can perform the operation.

- [ ] **Step 3: Implement the AppArmor LSM module**

Follow the existing capability and Yama module registration patterns. Register module name `apparmor`, include it in the complete module list and default optional list, and implement:

- task init: retain the builder-provided named default label;
- task clone: copy parent label to child;
- task exec: set `label_for_exec(current, executable_path)`;
- capability: convert the internal capability to its stable Linux number, call `decide_capability`, log denial fields, return `EPERM` only for `Decision::Deny`;
- ptrace: call `decide_ptrace` for tracer and tracee labels, log cross-profile denial fields, return `EPERM` only for `Decision::Deny`.

For `Decision::Complain`, emit the same structured denial with mode/result fields and return success. For `Decision::Allow`, do not emit a denial.

The log record must contain `apparmor="DENIED"`, operation, profile, task identifier, requested capability or tracee identifier, mode, and whether enforcement occurred.

- [ ] **Step 4: Run focused Rust checks**

Run:

```bash
cargo test -p aster-apparmor --lib
cargo check -p aster-core
```

Expected: both commands succeed without new warnings.

- [ ] **Step 5: Build the regression initramfs**

Run:

```bash
make -C test/initramfs ENABLE_REGRESSION_TEST=true initramfs-boot-images
```

Expected: the image builds and contains `/test/security/lsm/apparmor`.

- [ ] **Step 6: Run the repository's regression boot path**

Run the existing Asterinas regression boot command with the generated initramfs and default LSM selection. The AppArmor test must report success, and existing capability, module-selection, and Yama tests must remain successful.

- [ ] **Step 7: Review the security boundary**

Confirm the LSM result is an intersection with existing checks, cross-profile ptrace is never granted by AppArmor, unmatched exec cannot reset a confined label, and denial logging does not allocate or fail after the exec commit boundary.

---

## Final Review

Review the complete change against every acceptance criterion in the spec. Pay particular attention to exec rollback safety, child publication ordering, capability-number conversion, profile identity comparison, and accidental policy-loader scope growth.

Run the focused component test, core check, regression C build, and regression boot path once more only if a fix after Task 3 changed code covered by those checks.
