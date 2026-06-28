---
date: 2026-06-22
mode: diff
base: f4e29d67c
head: 1a2b3c4d
branch: demo-trapframe
title: "Remove TrapFrame::_pad"
---

<!--
  Illustrative output of the full pipeline on a realistic change (removing
  `TrapFrame::_pad`) — a teaching example of what a finished review file looks
  like, not a benchmark problem. Steps 1-5 (resolve -> activate -> fan out ->
  collect -> deterministic assemble) produced the body; step 6 (verification)
  confirmed every premise (no retractions); step 7 (consolidation) unified the
  seven comments — which all share one root cause across the Hardware and
  Security sections — into a single fix; step 8 wrote the Summary.
-->

# Summary

The change removes `TrapFrame::_pad` (and the matching assembly), tightening a
struct that does look like dead padding. It is, however, **unsound**: both the
Hardware and Security passes independently flag that the field is load-bearing for
the System V AMD64 16-byte stack alignment the CPU and `call trap_handler` rely
on. This is the one issue to fix, and it is **critical**.

All seven comments below share a single root cause — the `_pad` removal shrinks
`TrapFrame` from 176 to 168 bytes — so they have **one consolidated fix (C1)**:
revert the removal as a unit. Verification confirmed every premise (the size
arithmetic, the ABI requirement, and the deleted `const_assert!`).

> **Consolidated fix C1.** Restore the change as a unit: re-add `pub _pad: usize`
> to `TrapFrame`, restore `crate::const_assert!(size_of::<TrapFrame>().is_multiple_of(16))`
> and its SAFETY note, and on the kernel trap path restore `push 0` (entry) with
> the matching `add rsp, 24` (return). Keeping `size_of::<TrapFrame>()` a multiple
> of 16 is what holds RSP 16-byte aligned at `call trap_handler`.

---

## Security

### `ostd/src/arch/x86/trap/mod.rs` line 92

> ```diff
>      pub r15: usize,
> -    pub _pad: usize,
>  
>      pub trap_num: usize,
> ```

`justify-unsafe-use` (critical): Removing `_pad` shrinks `TrapFrame` to 168 bytes (not a multiple of 16). `trap.S` builds a `TrapFrame` on the stack then `call trap_handler` (a sysv64 boundary); the invariant is load-bearing for soundness — a misaligned RSP makes `trap_handler`'s SSE accesses fault, a kernel crash reachable via any kernel-mode interrupt/exception.

**Fix.** Shared fix **C1** (see Summary): restore `_pad` so `TrapFrame` stays 16-byte sized.

### `ostd/src/arch/x86/trap/mod.rs` line 94

> ```diff
> -crate::const_assert!(size_of::<TrapFrame>().is_multiple_of(16));
> ```

`justify-unsafe-use` (critical): The deleted `const_assert!` was a compile-time soundness guard enforcing the 16-byte-aligned-stack invariant `call trap_handler` relies on; removing it removes the only build-time mechanism that would catch the broken alignment.

**Fix.** Shared fix **C1** (see Summary): restore the `const_assert!` and its SAFETY note.

## Hardware

### `ostd/src/arch/x86/trap/mod.rs` line 92

> ```diff
>      pub r15: usize,
> -    pub _pad: usize,
>  
>      pub trap_num: usize,
> ```

`16b-align-rsp-before-call` (critical): Removing `pub _pad: usize` makes `size_of::<TrapFrame>()` = 168 bytes, no longer a multiple of 16. The kernel trap path materializes a `TrapFrame` on the stack before `call trap_handler`; the CPU 16-byte-aligns RSP on the interrupt, so a 168-byte frame leaves RSP misaligned by 8 at the call into Rust, violating the System V AMD64 ABI (UB for SSE such as `movaps`).

**Fix.** Shared fix **C1** (see Summary): keep the `_pad` field so the frame stays 176 bytes.

### `ostd/src/arch/x86/trap/mod.rs` line 94

> ```diff
> -crate::const_assert!(size_of::<TrapFrame>().is_multiple_of(16));
> ```

`16b-align-rsp-before-call` (major): The change deletes the only compile-time guard ensuring the trap-frame size keeps RSP 16-byte aligned at the call into Rust, removing the safety net that would catch exactly this regression.

**Fix.** Shared fix **C1** (see Summary): restore the `const_assert!` and its comment.

### `ostd/src/arch/x86/trap/trap.S` line 51

> ```diff
>      pop rax
> -    push 0
>      push r15
> ```

`16b-align-rsp-before-call` (critical): Removing `push 0` from `_trap_from_kernel` drops the 8 bytes that kept the bytes pushed before `call trap_handler` a multiple of 16, so RSP is 8-mod-16 at the call.

**Fix.** Shared fix **C1** (see Summary): restore `push 0` (with the matching `add rsp, 24` on return).

### `ostd/src/arch/x86/trap/trap.S` line 153

> ```diff
> -    add rsp, 16
> +    add rsp, 24
> ```

`16b-align-rsp-before-call` (major): The return path `add rsp, 16` (was `add rsp, 24`) is the matching half of the alignment-breaking change and must be reverted with it.

**Fix.** Shared fix **C1** (see Summary): revert to `add rsp, 24` together with the entry-side `push 0`.
