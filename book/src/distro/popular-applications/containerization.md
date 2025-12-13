# Containerization

## Podman

[Podman](https://docs.podman.io/en/stable/Introduction.html) is a modern, daemonless container engine
that provides a Docker-compatible command-line interface,
making it easy for users familiar with Docker to transition.

### Installation

To install Podman, add the following line to `configuration.nix`:

```nix
virtualization.podman.enable = true;
```

### Verified Usage

#### `podman run`

`podman run` runs a command in a new container.

```bash
# Start a container, execute a command, and then exit
podman run --name=c1 docker.io/library/alpine ls /etc

# Start a container and attach to an interactive shell
podman run -it docker.io/library/alpine
```

#### `podman image`

`podman image` manages local images.

```bash
# List downloaded images
podman image ls
```

#### `podman ps`

`podman ps` lists containers.

```bash
# Show the status of all containers (including exited ones)
podman ps -a
```

#### `podman rm`

`podman rm` removes one or more containers.

```bash
# Remove a container named foo
podman rm foo
```
