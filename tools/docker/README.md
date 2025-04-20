# Asterinas Development Docker Images

Asterinas development Docker images are provided to facilitate developing and testing Asterinas project. These images can be found in the [asterinas/asterinas](https://hub.docker.com/r/asterinas/asterinas/) repository on DockerHub.

## Building Docker Images

To build a Docker image for Asterinas and test it on your local machine, navigate to the root directory of the Asterinas source code tree and execute the following command:

```bash
cd <asterinas dir>
# Build Docker image
docker buildx build \
    -f tools/docker/Dockerfile \
    --build-arg ASTER_RUST_VERSION=$(grep "channel" rust-toolchain.toml | awk -F '"' '{print $2}') \
    -t asterinas/asterinas:$(cat VERSION)-$(date +%Y%m%d) \
    .
```

For the Intel TDX Docker image, it is based on a general Docker image. You can execute the following command:

```bash
cd <asterinas dir>
# Build Intel TDX Docker image
docker buildx build \
    -f tools/docker/tdx/Dockerfile \
    --build-arg ASTER_RUST_VERSION=$(grep "channel" rust-toolchain.toml | awk -F '"' '{print $2}') \
    --build-arg BASE_VERSION=${BASE_VERSION} \
    -t asterinas/asterinas:$(cat VERSION)-$(date +%Y%m%d)-tdx \
    .
```

Where `BASE_VERSION` represents the general Docker image you want to base it on.

## Tagging and Uploading Docker Images

Regarding the tagging Docker images, please refer to this [link](https://asterinas.github.io/book/to-contribute/version-bump.html).

New versions of Asterinas's Docker images are automatically uploaded to DockerHub through Github Actions. Simply submit your PR that updates Asterinas's Docker image for review. After getting the project maintainers' approval, the [Docker image building workflow](../../.github/workflows/publish_docker_images.yml) will be started, building the new Docker image and pushing it to DockerHub.
