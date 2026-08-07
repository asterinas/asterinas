# Asterinas Development Docker Images

Asterinas development Docker images are provided to facilitate developing and
testing the Asterinas project. These images can be found in the
[asterinas](https://hub.docker.com/r/asterinas/) repository
on DockerHub.

## Docker Image Dependencies

The images are built in the following order, with each image based on the
preceding one:

```text
asterinas/osdk-dev -> asterinas/prebuilt-nix-packages ->asterinas/kernel-dev -> asterinas/dev
```

`asterinas/osdk-dev` contains tools for building OSDK itself and developing
OSDK-based kernels. The later images add, respectively, Nix tooling and cached
Nix packages, Asterinas kernel-specific development tools, and agent tooling.

## Building Docker Images

Asterinas development Docker image is based on an OSDK development Docker image.
To build an Asterinas development Docker image and test it on your local
machine, navigate to the root directory of the Asterinas source code tree and
execute the following command:

```bash
cd <asterinas dir>
# Build Docker image
docker buildx build \
    -f tools/docker/Dockerfile \
    --platform linux/amd64,linux/arm64 \
    --build-arg ASTER_RUST_VERSION=$(grep "channel" rust-toolchain.toml | awk -F '"' '{print $2}') \
    --build-arg BASE_VERSION=$(cat DOCKER_IMAGE_VERSION) \
    -t asterinas/dev:$(cat DOCKER_IMAGE_VERSION) \
    .
```

## Tagging and Uploading Docker Images

The Docker images are tagged according to the version specified in the
`DOCKER_IMAGE_VERSION` file at the project root. Check out the
[version bump](https://asterinas.github.io/book/to-contribute/version-bump.html)
documentation on how new versions of the Docker images are released.
