# Package Management & Containerization

This category covers Nix package management tools and container runtimes.

## Package Management

### Nix

NixOS provides a set of [tools](https://nix.dev/manual/nix/2.28/command-ref/main-commands)
for building, installing, and managing packages.

#### Verified Usage

##### `nix-build`

`nix-build` builds Nix derivations and produces outputs in the Nix store.
It is the preferred way to build software reproducibly.

```bash
# Step 1: Create a clean workspace
rm -rf /tmp/nix-hello-c && mkdir -p /tmp/nix-hello-c && cd /tmp/nix-hello-c

# Step 2: Write a C program
cat > hello.c <<'EOF'
#include <stdio.h>

int main(void) {
    puts("Hello, World!");
    return 0;
}
EOF

# Step 3: Write a default.nix
cat > default.nix <<'EOF'
{ pkgs ? import <nixpkgs> {} }:

pkgs.stdenv.mkDerivation {
  pname = "hello-c";
  version = "0.1.0";
  src = ./.;

  dontConfigure = true;

  buildPhase = ''
    cc hello.c -o hello
  '';

  installPhase = ''
    mkdir -p $out/bin
    install -m755 hello $out/bin/hello
  '';
}
EOF

# Step 4: Build and run
nix-build
./result/bin/hello
```

##### `nix-env`

`nix-env` installs or removes individual packages in your user profile.

```bash
# Install the `hello` package
nix-env -iA nixos.hello

# Remove the `hello` package
nix-env -e hello
```

##### `nix-shell`

`nix-shell` creates a temporary development environment with the specified dependencies.
This is useful for testing software without modifying your system environment.

```bash
# Enter a shell with the `hello` package available
nix-shell -p hello
```

##### `nixos-rebuild`

`nixos-rebuild` manages the entire system configuration declaratively.
It applies changes defined in `configuration.nix`,
and is the recommended approach for installing packages system-wide.

```bash
# Edit the system configuration file
vim /etc/nixos/configuration.nix

# Apply configuration changes and rebuild the system without rebooting
nixos-rebuild test
```

<!--
TODO: upgrade mdbook to enable admonition blocks like the one below:

> [!WARNING]
> `nixos-rebuild switch` is not yet supported
-->

## Container Runtimes

### Podman

[Podman](https://docs.podman.io/en/stable/Introduction.html) is a modern, daemonless container engine
that provides a Docker-compatible command-line interface,
making it easy for users familiar with Docker to transition.

#### Installation

To install Podman, add the following line to `configuration.nix`:

```nix
virtualization.podman.enable = true;
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

### TODO: Docker

[Docker](https://www.docker.com/) is a widely-used container platform.

### TODO: containerd

[containerd](https://containerd.io/) is an industry-standard container runtime.

## Container Orchestration

### TODO: Kubernetes (kubectl)

[Kubernetes](https://kubernetes.io/) is a container orchestration platform.
`kubectl` is the command-line tool for interacting with Kubernetes clusters.

### TODO: Helm

[Helm](https://helm.sh/) is a package manager for Kubernetes.
