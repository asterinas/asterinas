## Motivation and pilot

AppArmor would give Asterinas a Linux-compatible application-confinement mechanism and a second end-to-end consumer of its LSM framework beyond capability and Yama.

The first QEMU pilot is intentionally small: one program runs under one profile, completes an allowed operation, and is denied one selected capability operation. Its label, errno, and audit result must be observable. The same image must retain the unconfined baseline when AppArmor is not selected. A real NixOS service or container workload is selected later for M5.

## Current Asterinas foundation

| Available | Missing |
| --- | --- |
| [<code>lsm=</code>/<code>security=</code> selection](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm/modules/mod.rs), mandatory capability, and optional Yama | Public registration/lifecycle seam for a high-level LSM component |
| [Capability and alien-access hooks](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm/hooks/mod.rs) | AppArmor task labels and exec/file hooks |
| Per-thread credentials and centralized exec prepare/commit | Binary policy decoder and AppArmor/securityfs nodes |
| <code>Path = Mount + Dentry</code>, common VFS paths, file handles, and mmap backing files | AppArmor audit delivery and compatibility tests |
| Boot parameters and QEMU regression tests | — |

The [kernel crate guidelines](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/book/src/to-contribute/coding-guidelines/for-maintainability/rust-specific/crates-and-modules.md) place high-level subsystems outside <code>aster-core</code> by default and require lower layers to define interfaces needed by higher layers.

## Proposed architecture

~~~text
kernel/core/src/security/lsm/          kernel/comps/apparmor/
--------------------------------      ------------------------------
generic hook contexts            ───▶ labels and profiles
registration/lifecycle seam           rule matcher and decisions
minimum opaque task/file state         ABI decoder and policy snapshot
clone/exec callbacks                   audit and hook adapter
                │
                └──── assembled by kernel/src; supported by distro/ and tests
~~~

~~~text
kernel operation → generic hook → AppArmor label/rule
                 → enforce or complain → audit → allow or errno
~~~

AppArmor is always an additional restriction: it cannot override a DAC or capability denial. Kernel code remains safe Rust, and GPL-only Linux/AppArmor implementation code is not copied or mechanically translated. The first seam change does not migrate capability or Yama.

| Built-in capability LSM | AppArmor capability rule |
| --- | --- |
| Checks whether the task credential contains the required capability | Checks whether the task's current profile permits using that capability |
| Mandatory authority check | Additional mandatory-access-control restriction |

Both must allow the operation. AppArmor cannot grant a missing capability, and possessing a capability does not bypass an AppArmor denial.

<details>
<summary>Why not put all AppArmor code in aster-core?</summary>

That would avoid a cross-crate seam but would move policy and ABI complexity into the core and conflict with the direction accepted in #3601. Keeping only generic hooks in core makes AppArmor replaceable and independently reviewable.

</details>

## Staged implementation roadmap

| Milestone | Deliverable | Exit gate |
| --- | --- | --- |
| **M1 — Runnable capability confinement** | Pin the contract; add the LSM seam and AppArmor component; decode/load the capability-only official blob subset; attach one profile at exec; enforce capability allow/deny with label inheritance, mode, audit, no unconfined fallback, and minimum ptrace safety. | In QEMU, a pinned-parser policy loads, the target enters its profile before its new image runs, an allowed operation succeeds, a selected capability operation is denied with the Linux-matched errno and audit record, and the unselected baseline is unchanged. |
| **M2 — File lifecycle** | Mediate open, create, mutation, descriptor transfer, mapping, and required qualifiers before side effects. | The advertised file group has no unmediated operation or qualifier. |
| **M3 — Exec transitions** | Extend M1's exact-path attachment with atomic <code>ix</code>, non-fallback <code>px</code>, cross-profile transitions, and fuller ptrace rules. | A new image runs only under its committed label and never falls back unconfined. |
| **M4 — Policy control plane** | Complete required securityfs ABI, atomic replace/remove, introspection, and audit delivery. | The pinned toolchain manages valid policy; invalid changes fail atomically; labels are observable. |
| **M5 — System pilot** | Build caches outside the kernel, load early, and confine one agreed workload through a non-fallback transition. | Behavior is reproducible in QEMU and documented as experimental. |

We especially welcome maintainer review of the LSM/component split and the capability-first M1.

<details>
<summary>References</summary>

- [Linux kernel AppArmor documentation](https://docs.kernel.org/admin-guide/LSM/apparmor.html)
- [AppArmor userspace project and parser](https://gitlab.com/apparmor/apparmor)
- [Asterinas LSM framework](https://github.com/asterinas/asterinas/tree/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/kernel/core/src/security/lsm)
- [Asterinas kernel crate guidelines](https://github.com/asterinas/asterinas/blob/34a8a2d72829897d8cab3b4cb9be8ceb69dac584/book/src/to-contribute/coding-guidelines/for-maintainability/rust-specific/crates-and-modules.md)
- [Asterinas RFC process](https://github.com/asterinas/asterinas/blob/main/book/src/rfcs/0001-rfc-process.md)
- [Asterinas kernel component proposal #3601](https://github.com/asterinas/asterinas/issues/3601)

</details>
