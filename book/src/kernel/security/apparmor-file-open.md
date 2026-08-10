# AppArmor File-Open Policy

Asterinas implements an exact-path allowlist for regular-file opens.
Policy loading and task confinement are manual.
The policy format described here is specific to Asterinas
and is not compatible with the Linux AppArmor policy ABI.

## Enable AppArmor

Enable AppArmor with the
[`lsm=` kernel parameter](../linux-compatibility/kernel-parameters.md#lsm):

```text
lsm=yama,apparmor
```

The legacy `security=apparmor` form is also accepted when `lsm=` is absent.

Mount `securityfs` after mounting `sysfs`:

```sh
mkdir -p /sys/kernel/security
mount -t securityfs none /sys/kernel/security
```

When AppArmor is active, its control files are available under `/sys/kernel/security/apparmor`.

## Load policy

One write may contain one version 0 profile.
A profile consists of a name followed by exact absolute-path rules:

```text
version 0
profile example
/etc/example.conf r
/var/lib/example/state rw
```

A policy write may contain at most 4096 bytes,
and a profile may contain at most 1024 rules.
A profile name may contain at most 128 bytes.
Profile names may contain ASCII letters, digits, `_`, `-`, and `.`.
`unconfined` is reserved and cannot be used as a profile name.
Rule paths must be canonical absolute paths without ASCII whitespace,
empty components, `.`, or `..`.
A rule path may contain at most 4096 bytes
and is matched against the path resolved by the VFS.

The supported permissions are:

| Permission | Allowed regular-file `file_open` operations |
| --- | --- |
| `r` | Open for reading |
| `w` | Open for writing or truncation |
| `rw` | Both sets above |

Write a new profile to `.load`,
or atomically replace an existing profile through `.replace`.
Writing either file requires `CAP_MAC_ADMIN` in the initial user namespace.
The `profiles` file lists loaded profiles,
and `features/policy_version` reports the supported policy version.

## Confine a task

Write a loaded profile name to `/proc/self/attr/current`,
then replace the task with the intended program:

```sh
sh -c 'printf %s example > /proc/self/attr/current; exec /path/to/program'
```

Only an unconfined task can enter a profile.
After entering a profile,
the task cannot change or remove it.
The label is part of the task credentials and is copied by `fork`.
It is preserved across `execve`.
Reading the attribute returns either `unconfined`
or the profile name followed by `(enforce)`.

## Enforcement

The VFS performs its usual checks before AppArmor,
so an AppArmor rule cannot grant access that the VFS denied.
A confined task receives `EACCES`
when it opens a regular file without a matching rule.
An `O_PATH` open requests no file permission and is allowed.

The `file_open` hook runs after creation.
If `O_CREAT` creates a file and AppArmor denies opening it,
the new directory entry remains.
An unnamed `O_TMPFILE` object is also checked after creation
and cannot match an exact absolute-path rule.

## Limitations

The hook covers only regular files opened through `open`, `openat`, or `creat`.
It does not check:

- directories, FIFOs, device nodes, or other non-regular files;
- file handles created by other system calls or by the kernel;
- existing file descriptions inherited across `fork` or `execve`;
- file descriptions received through `SCM_RIGHTS` or duplicated through
  `pidfd_getfd()`;
- later reads or writes through any of those existing descriptions.

As a result,
this implementation is not a complete confinement boundary
for descriptor-based access.
It also does not implement a create hook,
pathname globs,
profile removal,
automatic attachment,
complain mode,
execute transitions,
network rules,
or capability rules.
