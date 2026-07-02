# Nix Development Environment

The flake at the repository root provides a development shell as an
alternative to the [Docker-based environment](../tools/docker), whose images
layer as `osdk-dev` -> `prebuilt-nix-packages` -> `kernel-dev` -> `dev`. The
Rust toolchain comes from `rust-toolchain.toml`, the boot stack (QEMU, GRUB,
OVMF) is pinned to what `osdk-dev` builds, and the nixpkgs commit to the one
`prebuilt-nix-packages` pins. Lint and doc tools (typos, mdbook, ...) come
from nixpkgs and may lag the versions `osdk-dev` installs with
`cargo install`; `make check` inside the `kernel-dev` image is what CI runs.
klint is left to `osdk-dev`, since no build or check target invokes it.

With a flakes-enabled Nix, enter the dev shell from the repository root:

- Linux: `nix develop`: toolchain, QEMU, GRUB, OVMF; covers `make kernel` /
  `make run_kernel`. Projects scaffolded with `cargo osdk new` (and the OSDK
  test suite's TDX scheme) still expect the images' firmware paths, and the
  gvisor conformance tests need the `kernel-dev` image's prebuilt test
  binaries (point `GVISOR_PREBUILT_DIR` at a copy to run them elsewhere).
- macOS (Apple silicon): `nix develop`: build/lint subset (rustc, clippy,
  rustfmt, typos, mdbook, cross-building the no_std crates). Booting the
  kernel requires Linux.

The shell carries `rust-analyzer` from the same nightly as the toolchain.
Start your editor from within the shell (`nix develop`, then e.g. `code .`)
so it inherits the toolchain and `VDSO_LIBRARY_DIR`, which checking the
kernel crate requires.

Build a single dependency (Linux only): `nix build .#qemu` (also `.#grub`,
`.#ovmf`).
