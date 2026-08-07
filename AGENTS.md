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
docker run -it --privileged --network=host -v /dev:/dev \
  -v $(pwd)/asterinas:/root/asterinas \
  asterinas/dev:0.18.1-20260805
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

Set `TARGET_ARCH` to `x86_64` (default), `riscv64`, or `loongarch64`.

## Toolchain

- **Rust nightly** pinned in `rust-toolchain.toml`.
- **Edition:** 2024.
- `rustfmt.toml`: imports grouped as Std / External / Crate
  (`imports_granularity = "Crate"`, `group_imports = "StdExternalCrate"`).
- Clippy lints are configured in the workspace `Cargo.toml`
  under `[workspace.lints.clippy]`.
  Every member crate must have `[lints] workspace = true`.

## Coding Guidelines

The coding guidelines are the authoritative standard
for both **writing** and **reviewing** code.
The guidelines are organized by **persona**:
five durable engineering roles,
each a page whose Index doubles as that persona's review checklist.
Consult the persona whose concern matches your change.
Each Index lists every guideline as a stable `short-name` paired with a one-line gist,
so you can grasp a rule from the table and open its full text only when needed.

| Persona | Focus | Index |
|---|---|---|
| Project maintainer | Is the code well-shaped and understandable? | [For Maintainability](book/src/to-contribute/coding-guidelines/for-maintainability/README.md) |
| Kernel developer | Is it correct and efficient? | [For Development](book/src/to-contribute/coding-guidelines/for-development/README.md) |
| Security expert | Is it safe and secure? | [For Security](book/src/to-contribute/coding-guidelines/for-security/README.md) |
| Hardware expert | Is it correct against the hardware contract? | [For Hardware](book/src/to-contribute/coding-guidelines/for-hardware/README.md) |
| Documentation writer | Are the user-facing docs well-written? | [For Documentation](book/src/to-contribute/coding-guidelines/for-documentation/README.md) |

## Architecture Notes

Asterinas is a framekernel
comprising a safe upper half (`kernel/`)
and an unsafe lower half (`ostd/`).

The `ostd` crate is a minimal Rust OS development framework
that encapsulates `unsafe` Rust code within safe APIs.
`unsafe` Rust code is confined to OSTD (`ostd/`);
kernel code under `kernel/` must remain safe Rust.

The kernel code under `kernel/` is written using `ostd` APIs
and is organized as an acyclic graph of kernel crates
arranged in layers, from highest to lowest:

1. The assembler crate (`kernel/src/`).
2. High-level component crates (`kernel/comps/`).
3. The `aster-core` crate (`kernel/core/`).
4. Low-level component crates (`kernel/core/comps/`).
5. Kernel libraries (`kernel/libs/`).

A kernel crate may depend on crates in the same layer
or any lower layer.

## CI

CI runs in the project Docker container with KVM.
Key test matrices:
- x86-64: lint, compile, usermode tests, kernel tests, integration tests
  (boot, syscall, general), multiple boot protocols, SMP configurations.
- RISC-V 64, LoongArch 64, and Intel TDX have dedicated workflows.
- License headers and SCML validation are also checked.
