# Agents Guidelines for Asterinas

Asterinas is a Linux-compatible, general-purpose OS kernel
written in Rust using the framekernel architecture.
`unsafe` Rust is confined to OSTD (`ostd/`);
the kernel (`kernel/`) is entirely safe Rust.

## Repository Layout

| Directory    | Purpose                                                  |
|--------------|----------------------------------------------------------|
| `kernel/`    | Safe-Rust OS kernel (syscalls, VFS, networking, etc.)    |
| `ostd/`      | OS framework — the only crate permitted to use `unsafe`  |
| `osdk/`      | `cargo-osdk` CLI tool for building/running/testing       |
| `test/`      | Regression and syscall tests (C user-space programs)     |
| `distro/`    | Asterinas NixOS distribution configuration               |
| `tools/`     | Utility scripts (formatting, Docker, benchmarking, etc.) |
| `book/`      | The Asterinas Book (mdBook documentation)                |

## Building and Running

All development is done inside the project Docker container:

```bash
docker run -it --privileged --network=host --device=/dev/kvm -v /dev:/dev \
  -v $(pwd)/asterinas:/root/asterinas \
  asterinas/asterinas:0.17.0-20260227
```

Key Makefile targets:

| Command              | What it does                                         |
|----------------------|------------------------------------------------------|
| `make kernel`        | Build initramfs and the kernel                       |
| `make run_kernel`    | Build and run in QEMU                                |
| `make test`          | Unit tests for non-OSDK crates (`cargo test`)        |
| `make ktest`         | Kernel-mode unit tests via `cargo osdk test` in QEMU |
| `make check`         | Full lint: rustfmt, clippy, typos, license checks    |
| `make format`        | Auto-format Rust, Nix, and C code                    |
| `make docs`          | Build rustdocs for all crates                        |

Set `OSDK_TARGET_ARCH` to `x86_64` (default), `riscv64`, or `loongarch64`.

## Toolchain

- **Rust nightly** pinned in `rust-toolchain.toml` (nightly-2025-12-06).
- **Edition:** 2024.
- `rustfmt.toml`: imports grouped as Std / External / Crate
  (`imports_granularity = "Crate"`, `group_imports = "StdExternalCrate"`).
- Clippy lints are configured in the workspace `Cargo.toml`
  under `[workspace.lints.clippy]`.
  Every member crate must have `[lints] workspace = true`.

## Coding Guidelines

The full coding guidelines live in
`book/src/to-contribute/coding-guidelines/`.
Below is a condensed summary of the most important rules.

### General

- **Be descriptive.** No single-letter names or ambiguous abbreviations.
- **Explain why, not what.** Comments that restate code are noise.
- **One concept per file.** Split when files grow long.
- **Organize for top-down reading.** High-level entry points first.
- **Hide implementation details.** Narrowest visibility by default.
- **Validate at boundaries, trust internally.**
  Validate at syscall entry; trust already-validated values inside.

### Rust

- **Naming:** CamelCase with title-cased acronyms (`IoMemoryArea`).
  Closure variables end in `_fn`.
- **Functions:** Keep small and focused; minimize nesting (max 3 levels);
  use early returns, `let...else`, and `?`.
  Avoid boolean arguments — use an enum or split into two functions.
- **Types:** Use types to enforce invariants.
  Prefer enums over trait objects for closed sets.
  Encapsulate fields behind getters.
- **Unsafety:**
  - Every `unsafe` block requires a `// SAFETY:` comment.
  - Every `unsafe fn` or `unsafe trait` requires a `# Safety` doc section.
  - All crates under `kernel/` must have `#![deny(unsafe_code)]`.
    Only `ostd/` may contain unsafe code.
- **Modules:** Default to `pub(super)` or `pub(crate)`;
  use `pub` only when truly needed.
  Always use `workspace.dependencies`.
- **Error handling:** Propagate errors with `?`.
  Do not `.unwrap()` where failure is possible.
- **Logging:** Use `log` crate macros only (`trace!`..`error!`).
  No `println!` in production code.
- **Concurrency:** Establish and document lock order.
  Never do I/O or blocking under a spinlock.
  Avoid casual use of atomics.
- **Performance:** Avoid O(n) on hot paths.
  Minimize unnecessary copies, allocations, and `Arc::clone`s.
  No premature optimization without benchmark evidence.
- **Macros and attributes:** Prefer functions over macros.
  Suppress lints at the narrowest scope.
  Prefer `#[expect(...)]` over `#[allow(...)]`.
- **Doc comments:** First line uses third-person singular present
  ("Returns", "Creates"). End sentence comments with punctuation.
  Wrap identifiers in backticks.
- **Arithmetic:** Use checked or saturating arithmetic.

### Git

- **Subject line:** Imperative mood, at or below 72 characters.
  Common prefixes: `Fix`, `Add`, `Remove`, `Refactor`, `Rename`,
  `Implement`, `Enable`, `Clean up`, `Bump`.
- **Atomic commits:** One logical change per commit.
- **Separate refactoring from features** into distinct commits.
- **Focused PRs:** One topic per PR. Ensure CI passes before review.

### Testing

- **Add regression tests for every bug fix** (with issue reference).
- **Test user-visible behavior** through public APIs, not internals.
- **Use assertion macros**, not manual output inspection.
- **Clean up resources** after every test (fds, temp files, child processes).

### Assembly

- Use `.balign` over `.align` for unambiguous byte-count alignment.
- Add `.type` and `.size` for Rust-callable functions.
- Use unique label prefixes to avoid name clashes in `global_asm!`.

## Architecture Notes

- **Framekernel:** The kernel is split into a safe upper half (`kernel/`)
  and an unsafe lower half (`ostd/`).
  This is a hard architectural boundary — never add `unsafe` to `kernel/`.
- **Components** (`kernel/comps/`): block, console, network, PCI, virtio, etc.
  Each is a separate crate.
- **OSTD** (`ostd/`): memory management, page tables, interrupt handling,
  synchronization primitives, task scheduling, boot, and arch-specific code.
- **Architectures:** x86-64 (primary), RISC-V 64, LoongArch 64.
  Arch-specific code lives in `ostd/src/arch/` and `kernel/src/arch/`.

## CI

CI runs in the project Docker container with KVM.
Key test matrices:
- x86-64: lint, compile, usermode tests, kernel tests, integration tests
  (boot, syscall, general), multiple boot protocols, SMP configurations.
- RISC-V 64, LoongArch 64, and Intel TDX have dedicated workflows.
- License headers and SCML validation are also checked.
