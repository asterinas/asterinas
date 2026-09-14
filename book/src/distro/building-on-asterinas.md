# Building Asterinas on Asterinas

The `x86_64-linux` self-host path uses the normal AsterNixOS image:

- **L0 — Linux host:** builds the bootstrap kernel and the normal AsterNixOS
  disk, then runs the outer QEMU with KVM.
- **L1 — AsterNixOS self-host guest:** clones the source at runtime, enters
  the repository flake with `nix develop`, and builds Asterinas.
- **L2 — nested Asterinas guest:** boots the L1-built kernel with QEMU/TCG.

The normal AsterNixOS configuration provides the compatibility needed by Nix:

- disabled Nix sandboxing, syscall filtering, and build users, because their
  required isolation mechanisms are not yet supported;
- a high file-descriptor limit, so large closure realizations do not exhaust
  the 1,024-descriptor default.

Nix reads `flake.lock`, evaluates the development shell, and queries the
configured substituters for the resulting paths.

## Build and boot L1

```bash
nix develop --command make iso TARGET_ARCH=x86_64
nix develop --command make run_iso TARGET_ARCH=x86_64 NIXOS_DISK_SIZE_IN_MB=65536
```

The dev shell exports `LINUX_EFI_GRUB_MKRESCUE`, because the kernel build
otherwise expects the Docker image's grub-mkrescue path.

The installation needs 64 GiB of free disk: `run_iso` writes the image file
in full (it is not sparse).

In the Docker environment from [Getting Started](README.md), the host
installer is available instead: `make nixos NIXOS_DISK_SIZE_IN_MB=65536`
writes the same installed disk directly (the container has the root and
loop-device access the installer needs), so `make iso` / `make run_iso`
are not needed there. The boot command below is the same either way.

Boot the installed disk with L1 build resources:

```bash
nix develop --command make run_nixos MEM=32G SMP=8 TARGET_ARCH=x86_64
```

Use `MEM=32G SMP=8` for the L1 build. The Makefile defaults (8 GiB and one
vCPU) are sufficient to boot, but a 24 GiB guest exhausted kernel memory
during the build.

L1 runs the full AsterNixOS userspace: systemd is PID 1, and the hvc0
console logs in a root shell automatically. Typing `exit` only ends the
session; getty starts a new one. Asterinas currently has no background
writeback. Remember to run `sync` periodically and before stopping the VM
to flush pending writes to disk. To reset the disk, rerun `make run_iso` rather
than deleting multi-gigabyte trees inside the guest.

## Transfer the source

The image does not include the Asterinas source. Clone it into a writable
directory inside L1; a shallow clone is sufficient:

```bash
git clone --depth 1 https://github.com/asterinas/asterinas /work/asterinas
```

Keep the checkout as a Git repository. Otherwise, Nix may treat the flake as
a plain path and copy ignored build output such as `target/` into the store.

## Build at L1

Inside L1, use the flake directly:

```bash
cd /work/asterinas
nix develop --option http-connections 1
```

A previous large-closure realization with the default parallel transfers hit
Nix's 300-second stalled-download timeout, while a single connection completed.

Then, in the development shell:

```bash
make kernel TARGET_ARCH=x86_64
```

`flake.lock` pins the Nix toolchain and system dependencies, while the Cargo
lockfiles pin Rust registry and Git dependencies. Nix downloads available
binary-cache artifacts and builds missing store paths locally. Cargo fetches
its locked dependencies normally. Make manages the test-disk images and
reuses existing images until `make clean` removes them.

## Boot L2 with TCG

In the L1 development shell:

```bash
make run_kernel \
  TARGET_ARCH=x86_64 \
  ENABLE_KVM=0 \
  NETDEV=none \
  QEMU_DISPLAY=none \
  MEM=2G \
  SMP=1 \
  AUTO_TEST=boot
```

L2 runs under TCG with one vCPU and 2 GiB of RAM. It uses the default GRUB
rescue ISO and the Nix-provided OVMF firmware, without network or display.
`AUTO_TEST=boot` fails unless `qemu.log` contains
`Successfully booted.`

## Limitations

- Only `x86_64-linux` has been validated.
- L1 has outbound access through QEMU user-mode networking. L2 networking is
  disabled because QEMU's default `hostfwd` listeners cannot bind inside
  Asterinas.
- AsterNixOS disables Nix sandboxing, syscall filtering, and build users.
- L2 uses TCG; nested hardware acceleration is not used.
