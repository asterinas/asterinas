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
podman run --name=c1 docker.io/library/alpine ls /etc

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
