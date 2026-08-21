# Building Asterinas on Asterinas

Asterinas is capable of _self-hosting_:
the tools that produce the kernel can run on the kernel itself.
This page demonstrates how one can use Asterinas NixOS
to build Asterinas from source
and boot the freshly built kernel in a nested VM.
Along the way, Nix, Cargo, rustc, and QEMU all run on Asterinas,
which makes a self-hosted build one of the largest real-world workloads
the kernel has been put through.

This capability is still experimental.
The procedure on this page has been verified only on x86-64,
and only up to the point where the nested kernel reports a successful boot.

## Before you begin

You need an x86-64 Linux host with KVM.
On the host, set up either the Docker container from
[Getting Started](../kernel/#getting-started)
or the [Nix development shell](../kernel/nix-development.md),
and run the host commands on this page inside it.
The host needs about 50 GiB of free disk space,
enough memory to give the VM 40 GiB,
and Internet access,
because the VM downloads the source, Nix packages, and Rust crates.

Depending on the host's performance,
expect the installation to take about 30 minutes
and the first build inside the VM to take more than an hour.

## Install Asterinas NixOS

Install Asterinas NixOS as described for
[kernel developers](../distro/#kernel-developers),
but with a 32 GiB disk.
The default 16 GiB disk leaves too little room.
The development shell alone takes about 6 GiB,
and after the first build the VM uses about 18 GiB.

In the Docker container, run:

```bash
make nixos NIXOS_DISK_SIZE_IN_MB=32768
```

The [Nix development shell](../kernel/nix-development.md#enter-the-development-shell)
runs without root privileges,
and `make nixos` needs them to set up a loop device,
so in the shell, install from the ISO image instead:

```bash
make iso
make run_iso NIXOS_DISK_SIZE_IN_MB=32768
```

`make run_iso` installs Asterinas NixOS in a VM without further input
and exits when the installation finishes.

## Start the VM

Boot the installed disk with 40 GiB of memory and four vCPUs:

```bash
make run_nixos MEM=40G SMP=4
```

Do not give the VM less memory.
Asterinas does not yet free cached file data under memory pressure,
so everything the VM downloads or builds stays in memory,
and the VM uses about 31 GiB by the end of `make kernel`.
With 32 GiB and eight vCPUs,
the kernel sometimes ran out of memory near the end of the build.

The console logs in as root automatically.
To stop the VM, run `sync` and then `poweroff`.
Typing `exit` does not stop it, because the console logs in again.

## Get the source

The image does not include the Asterinas source.
Clone it inside the VM:

```bash
git clone --depth 1 https://github.com/asterinas/asterinas
```

The VM does not share files with the host.
To build your own changes, push them to a branch the VM can reach,
such as one in your fork, and clone that branch with `--branch`.

Use `git clone` rather than downloading a source archive.
Nix copies only tracked files from a Git checkout into its store.
From a plain directory, it copies everything,
including ignored build output such as `target/`.

## Build the kernel inside the VM

Enter the development shell from the checkout:

```bash
cd asterinas
nix develop --accept-flake-config
```

The flake declares the project's binary caches on Cachix.
Without `--accept-flake-config`, Nix first asks whether to accept them.
The image already uses the same caches, so the answer changes nothing.

The first run downloads about 6 GiB of packages
and builds the ones that no binary cache provides.
Then build the kernel:

```bash
make kernel
```

## Boot the kernel you built

In the development shell inside the VM,
boot the new kernel in a nested VM:

```bash
make run_kernel \
  ENABLE_KVM=0 \
  NETDEV=none \
  QEMU_DISPLAY=none \
  MEM=2G \
  AUTO_TEST=boot
```

Asterinas does not provide KVM,
so `ENABLE_KVM=0` runs the nested VM under QEMU's software emulation,
and booting takes a few minutes.
`NETDEV=none` and `QEMU_DISPLAY=none` turn off networking and VNC,
because QEMU cannot open their listening sockets inside Asterinas.
`MEM=2G` keeps the nested VM small.

With `AUTO_TEST=boot`, the new kernel prints `Successfully booted.` once it boots,
and the nested VM then exits on its own.
`make run_kernel` itself does not return on Asterinas yet.

## Limitations

- Only the boot test has been run in the nested VM.
  The nested VM has no network or display,
  so tests that need them cannot run there.
- The nested VM runs under software emulation,
  so even the boot test takes a few minutes.
- Nix inside the VM builds as root and without a sandbox,
  because Asterinas does not yet support the isolation that Nix relies on.
  Builds are therefore not isolated from the rest of the system.
