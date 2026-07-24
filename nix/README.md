# Nix Development Environment

The flake at the repository root provides a development shell as an
alternative to the [Docker-based environment](../tools/docker). The Rust
toolchain (from `rust-toolchain.toml`), the boot stack (QEMU, GRUB, OVMF,
klint), and the nixpkgs commit are pinned to the versions the Docker image
uses; each pin lives next to a comment naming its Dockerfile counterpart, so
bump them together. Lint and doc tools (typos, mdbook, ...) come from nixpkgs
and may lag the versions the Docker image installs with `cargo install`;
`make check` inside the Docker image is what CI runs.

With a flakes-enabled Nix, enter the dev shell from the repository root:

- Linux: `nix develop`: toolchain, QEMU, GRUB, OVMF; covers `make kernel` /
  `make run_kernel`. Projects scaffolded with `cargo osdk new` (and the OSDK
  test suite's TDX scheme) still expect the Docker image's firmware paths,
  and the gvisor conformance tests need the Docker image's prebuilt test
  binaries (point `GVISOR_PREBUILT_DIR` at a copy to run them elsewhere).
- macOS (Apple silicon): `nix develop`: build/lint subset (rustc, clippy,
  rustfmt, typos, mdbook, cross-building the no_std crates). Booting the
  kernel requires Linux.

The shell carries `rust-analyzer` from the same nightly as the toolchain.
Start your editor from within the shell (`nix develop`, then e.g. `code .`)
so it inherits the toolchain and `VDSO_LIBRARY_DIR`, which checking the
kernel crate requires.

Build a single dependency (Linux only): `nix build .#qemu` (also `.#grub`,
`.#ovmf`, `.#klint`).

## Entering the shell automatically

With [direnv](https://direnv.net/) installed, the shell loads on `cd` instead
of an explicit `nix develop`. Approve the `.envrc` that ships with the
repository once:

```sh
direnv allow
```

direnv binds that approval to the contents of `.envrc`, so a `git pull` that
changes the file blocks it until you approve it again. `.envrc` also declares
the dev shell sources as watched inputs: neither direnv's `use flake` nor
nix-direnv's replacement looks beyond `flake.nix` and `flake.lock`, so without
those declarations an edit under `nix/` leaves you in the previously cached
shell.
