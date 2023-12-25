# Asterinas Development Docker Images

Asterinas development Docker images are provided to facilitate developing and testing Asterinas project. These images can be found in the [asterinas/asterinas](https://hub.docker.com/r/asterinas/asterinas/) repository on DockerHub.

## Building Docker Images

To build a Docker image for Asterinas and test it on your local machine, navigate to the root directory of the Asterinas source code tree and execute the following command:

```bash
docker buildx build \
    -f tools/docker/Dockerfile.ubuntu22.04 \
    --build-arg ASTER_RUST_VERSION=$RUST_VERSION \
    -t asterinas/asterinas:$ASTER_VERSION \
    .
```

The meanings of the two environment variables in the command are as follows:

- `$ASTER_VERSION`: Represents the version number of Asterinas. You can find this in the `VERSION` file.
- `$RUST_VERSION`: Denotes the required Rust toolchain version, as specified in the `rust-toolchain` file.

## Tagging Docker Images

It's essential for each Asterinas Docker image to have a distinct tag. By convention, the tag is assigned with the version number of the Asterinas project itself. This methodology ensures clear correspondence between a commit of the source code and its respective Docker image.

If a commit needs to create a new Docker image, it should

1. Update the Dockerfile as well as other materials relevant to the Docker image, and
2. Run [`tools/bump_version.sh`](../bump_version.sh) tool to update the Asterinas project's version number.
 
For bug fixes or small changes, increment the last number of a [SemVer](https://semver.org/) by one. For major features or releases, increment the second number. All changes made in the two steps should be included in the commit.

## Uploading Docker Images

New versions of Asterinas's Docker images are automatically uploaded to DockerHub through Github Actions. Simply submit your PR that updates Asterinas's Docker image for review. After getting the project maintainers' approval, the [Docker image building workflow](../../.github/workflows/docker_build.yml) will be started, building the new Docker image and pushing it to DockerHub.