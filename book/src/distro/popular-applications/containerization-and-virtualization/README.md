# Containerization and Virtualization

This category covers container runtimes, container image tools, and other virtualization-related tools.

## Container Runtimes

### Podman

[Podman](https://docs.podman.io/en/stable/Introduction.html) is a modern, daemonless container engine
that provides a Docker-compatible command-line interface,
making it easy for users familiar with Docker to transition.

#### Installation

To install Podman, add the following line to `configuration.nix`:

```nix
virtualisation.podman.enable = true;
```

#### Verified Usage

##### `podman run`

`podman run` runs a command in a new container.

```bash
# Start a container, execute a command, and then exit
podman run --name=podman-example docker.io/library/alpine ls /etc

# Start a container and attach to an interactive shell
podman run -it docker.io/library/alpine
```

##### `podman image`

`podman image` manages local images.

```bash
# List downloaded images
podman image ls
```

##### `podman ps`

`podman ps` lists containers.

```bash
# Show the status of all containers (including exited ones)
podman ps -a
```

##### `podman rm`

`podman rm` removes one or more containers.

```bash
# Remove a container named foo
podman rm foo
```

### Kata Containers

[Kata Containers](https://katacontainers.io/) runs containers inside lightweight virtual machines.
The setup below uses containerd's `ctr` command to run an x86-64 container
with QEMU's TCG software emulation.
The inner Kata VM does not require KVM.
Container networking and Kubernetes integration are disabled.
Use this configuration only for evaluation with trusted images:
virtiofsd's host sandbox is disabled and host resource limits are not enforced reliably.

#### Prerequisites

Kata uses Asterinas mount namespaces.
In addition, this experimental setup requires an Asterinas kernel build with:

- [Shared/slave mount propagation](https://github.com/asterinas/asterinas/pull/3639),
  which Kata uses to share the container's root filesystem with the virtual machine; and
- [the common vhost layer](https://github.com/asterinas/asterinas/pull/3699)
  and [the host vhost-vsock device](https://github.com/asterinas/asterinas/pull/3808)
  built on it, including AF_VSOCK connections to the guest,
  which Kata uses to communicate with its agent inside the virtual machine.

This configuration is experimental and has not passed an end-to-end test on the current main branch.
Earlier validation used a patched kernel with a standalone vhost-vsock backend;
it does not establish compatibility with the common vhost layer and its new device backend.
The NixOS module configures user-space packages and cannot supply missing kernel support.

The package overrides target Kata Containers 3.29.0 and the Nixpkgs revision
supplied by Asterinas NixOS.
Earlier validation used containerd 2.3.0, QEMU 10.2.4, and virtiofsd 1.13.3.

#### Installation

After adding the required kernel support, enable the experimental module in `configuration.nix`:

```nix
aster_nixos.kata-tcg.enable = true;
```

The module is included in Asterinas NixOS.
The integration test enables the same option, so its runtime settings match this example.

The module installs a patched Kata runtime, configures containerd,
and places the Kata guest image under `/var/lib/kata-containers`.
It selects one virtual CPU and 256 MiB of guest memory.

Run the following commands as root to apply the configuration and check the service:

```bash
nixos-rebuild switch
systemctl restart containerd
systemctl is-active containerd
test -S /run/containerd/containerd.sock
test -c /dev/vhost-vsock
```

The service should report `active`,
and both `test` commands should exit successfully.
Run the container commands below as root as well.

#### Experimental Usage

##### Import an image

The integration test supplies a locally built static BusyBox image archive.
Copy an uncompressed OCI or Docker image archive to Asterinas as `busybox.tar`,
then import and unpack it with the `native` snapshotter:

```bash
ctr images import --local --snapshotter native \
  --platform linux/amd64 busybox.tar
ctr images ls
```

The run command below assumes the archive contains an image named
`docker.io/library/busybox:latest` with `/bin/sh`.
If your archive uses a different name, use the name shown by `ctr images ls`.

The `native` snapshotter copies image layers instead of using OverlayFS.
OCI images remain usable without host OverlayFS,
but unpacking requires more disk space and copying.
Select `native` for both image import and container execution.

##### Run a container

```bash
ctr run --rm --snapshotter native \
  --runtime io.containerd.kata.v2 \
  docker.io/library/busybox:latest kata-example \
  /bin/sh -c 'echo hello-from-kata'
```

The command should print `hello-from-kata` and exit successfully.
The `--rm` flag removes the container after it exits.
Check that `kata-example` no longer appears in:

```bash
ctr containers ls
```

#### Compatibility Settings and Limitations

The NixOS module applies the following settings and Kata patches.
Containerd itself is not patched.

| Component | Configuration or workaround | Effect |
| --- | --- | --- |
| QEMU | Patch Kata to select TCG and the `max` CPU model; skip KVM device discovery | Runs without `/dev/kvm`. These overrides are specific to TCG. |
| containerd | Disable `io.containerd.grpc.v1.cri` | Use `ctr` directly. This setup does not support Kubernetes. |
| Kata networking | Set `internetworking_model = "none"`, `disable_new_netns = true`, and `disable_vhost_net = true` | No container network or host network-namespace setup. |
| virtiofsd | Set `--sandbox=none`, `--seccomp=none`, and `--inode-file-handles=never` | Avoids unsupported namespace, seccomp, and file-handle operations, but disables virtiofsd's own namespace sandbox and seccomp filter. |
| Host cgroups | Ignore initial resource-controller setup errors and set `sandbox_cgroup_only = true` | Host cgroup limits and accounting for the Kata shim and QEMU are not guaranteed. |
| Logging | Skip Kata's syslog initialization and omit virtiofsd's `--syslog` option | These syslog records are unavailable. |

The Kata shim still needs a mount namespace and working mount propagation;
the module does not bypass them.
The TCG patch preserves ordinary and vhost device ACL setup while removing hardware-accelerator discovery.
Kata configuration parsing and validation still run when syslog initialization is skipped.
Registry downloads, container networking, host resource isolation, and performance are outside this profile.

Only the gRPC CRI plugin is disabled.
Leave containerd's internal `io.containerd.cri.v1.images` and
`io.containerd.cri.v1.runtime` plugins enabled,
as other containerd services depend on them.

#### Troubleshooting

Some `ctr` commands may print `Failed to check deprecations`
with an error mentioning `/proc/<pid>/ns/pid`.
This is a host PID-namespace information query;
it did not prevent image import, execution, or container removal in the earlier standalone-backend validation.
Check the command's exit status and output rather than treating this warning as a failure.

To skip this automatic query, set the following in the shell where you run `ctr`:

```bash
export CONTAINERD_SUPPRESS_DEPRECATION_WARNINGS=true
```

This existing containerd option requires no patch.
It also suppresses other deprecation notices printed by `ctr`.
It does not add PID namespace support or fix explicit introspection queries.

`ctr version` also uses this query and may fail.
Use `systemctl is-active containerd` and the socket check above to check readiness;
use `containerd --version` to inspect the installed binary's version.

#### Integration Test

After integrating the kernel dependencies, build a new image before running the test:

```bash
make nixos NIXOS_TEST_SUITE=kata-tcg NIXOS_DISK_SIZE_IN_MB=16384
make run_nixos NIXOS_TEST_SUITE=kata-tcg \
  NIXOS_TEST_CASE=kata_tcg_native_snapshotter
```

The test imports the offline image, checks the inner QEMU arguments for TCG,
checks the container output and exit status, and checks that its container record was removed.
It does not check complete resource reclamation or measure performance.
`ENABLE_KVM` controls the outer QEMU that boots Asterinas;
it is independent of this module's inner TCG selection.

## Container Image Tools

### Skopeo

[Skopeo](https://github.com/containers/skopeo) inspects and copies container images without a daemon.

#### Installation

```nix
environment.systemPackages = [ pkgs.skopeo ];
```

#### Verified Usage

```bash
# Inspect a remote image
skopeo inspect docker://docker.io/library/alpine:latest

# List all tags for a repository
skopeo list-tags docker://docker.io/library/alpine
```

## Virtualization

### QEMU

[QEMU](https://www.qemu.org/) is the most widely used open-source machine emulator and virtualizer. It supports full system emulation as well as user-mode binary translation.

Asterinas does not yet support hardware-assisted virtualization (KVM), therefore QEMU runs exclusively with **TCG** (Tiny Code Generator / software emulation) on Asterinas NixOS.

#### Installation

```nix
environment.systemPackages = with pkgs; [ qemu_kvm ];

environment.variables = {
  LINUX_BZIMAGE = "${pkgs.linuxPackages.kernel}/bzImage";
  OVMF_PATH = "${pkgs.OVMF.fd}/FV/OVMF.fd";
};
```

#### Environment Variables

The following environment variables are automatically provided **when building the NixOS test suite**:

- `LINUX_BZIMAGE`: Path to the standard Linux kernel bzImage
- `OVMF_PATH`: Path to the OVMF (UEFI) firmware

You can enable them by building with:

```bash
make nixos NIXOS_TEST_SUITE=containerization-and-virtualization
```

#### Verified Usage

##### Display QEMU version

```bash
qemu-system-$(uname -m) --version
```

##### Run Linux kernel with TCG

```bash
qemu-system-$(uname -m) \
  -accel tcg \
  -kernel $LINUX_BZIMAGE \
  -initrd /run/current-system/initrd \
  -nographic -no-reboot \
  -append 'console=ttyS0 panic=-1 rdinit=/bin/init'
```

##### Run Asterinas kernel with TCG

```bash
qemu-system-$(uname -m) \
  -accel tcg \
  -cpu Icelake-Server \
  -machine q35 -m 1G \
  -bios $OVMF_PATH \
  -kernel /run/current-system/kernel \
  -initrd /run/current-system/initrd \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -nographic -no-reboot \
  -append 'console=ttyS0 panic=-1 rdinit=/bin/init'
```

> **Note**: Running the Asterinas kernel requires the `linux/multiboot` boot protocol (**multiboot2 is not supported**).
> Compile Asterinas with:
> ```bash
> make nixos BOOT_PROTOCOL=linux NIXOS_TEST_SUITE=containerization-and-virtualization
> ```
