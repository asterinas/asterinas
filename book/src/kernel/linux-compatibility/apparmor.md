# AppArmor

Asterinas provides an AppArmor-like major Linux Security Module (LSM).
It can be enabled with either `security=apparmor` or an `lsm=` list that
contains `apparmor`.

The implementation follows the Linux AppArmor model in the areas that are
currently supported:
- AppArmor confinement is attached to task credentials.
- Tasks run under a current label/profile and inherit that state across
  fork-like task creation.
- Policy is loaded from user space instead of being hard-coded in the kernel.
- File access is mediated by matching resolved paths against profile policy.
- Capability use is mediated through the kernel capability hook.
- AppArmor task identity is exposed through procfs.

This is a supported AppArmor subset, not a claim of full Linux AppArmor policy
ABI coverage.

## Policy management

When AppArmor is enabled, mount securityfs to access the AppArmor policy
management interface:

```sh
mount -t securityfs none /sys/kernel/security
```

The AppArmor directory is then available at:

```text
/sys/kernel/security/apparmor
```

Supported policy control files:
- `.load`: loads policy data.
- `.replace`: loads or replaces policy data.
- `.remove`: removes policy data.
- `profiles`: lists loaded profiles and their modes.
- `features`: exposes the AppArmor feature ABI supported by Asterinas.

The `.load` and `.replace` files accept binary policy data produced by user
space. The `.remove` file accepts either a binary remove payload or a profile
name. Policy control writes require `CAP_MAC_ADMIN` in the initial user
namespace.

The feature ABI currently exposes:
- `features/abi`
- `features/policy/versions/{v5,v6,v7,v8,v9}`
- `features/policy/set_load`
- `features/policy/permstable32`
- `features/policy/permstable32_version`
- `features/file/mask`
- `features/caps/mask`
- `features/domain/change_profile`
- `features/domain/change_onexec`
- `features/domain/version`

The `features/abi` file reports the supported Asterinas AppArmor ABI subset,
including the Linux policy ABI range, file audit/quiet support, capability
audit support, and complain-mode support.

## Procfs interfaces

AppArmor task identity is exposed through:

```text
/proc/[pid]/task/[tid]/attr/current
/proc/[pid]/task/[tid]/attr/exec
/proc/[pid]/task/[tid]/attr/prev
```

The files report the current profile, the on-exec profile, and the previous
profile for the task. The `current` and `exec` files are writable by the
current task to request profile changes supported by the loaded policy.

Asterinas also exposes compatibility control files under:

```text
/proc/sys/kernel/apparmor
```

Supported entries:
- `profiles`
- `load`
- `current`

## Mediation coverage

The current implementation mediates file and capability operations.

File mediation covers path-based operations including:
- open
- create
- delete
- link
- rename
- setattr
- file permission checks
- mmap
- file receive
- file lock
- getattr
- exec

Capability mediation covers capability checks that flow through
`security::capable`.

Network, mount, ptrace, signal, D-Bus, and rlimit mediation are not covered by
this AppArmor subset yet.

## Policy behavior

Profiles may run in enforce or complain mode.
In enforce mode, denied accesses fail.
In complain mode, implicit denials are recorded through the AppArmor decision
path but are allowed to continue. Explicit deny rules remain enforced.

The file policy is conservative: an access is allowed only when the current
profile grants the requested permissions, and explicit deny rules take
precedence over allow rules.
