# Version Bump

## Version Numbers

Currently, Asterinas regularly releases two main artifacts
for the Rust OS developer community:
the [OSDK](https://crates.io/crates/cargo-osdk) and [OSTD](https://crates.io/crates/ostd).
To support development with these tools,
we also publish companion Docker images on DockerHub,
(i.e., [`asterinas/osdk`](https://hub.docker.com/r/asterinas/osdk)).
While the Asterinas kernel is not yet ready for public binary release,
its development Docker images
(i.e., [`asterinas/asterinas`](https://hub.docker.com/r/asterinas/asterinas))
are released regularly.

All released crates for OSDK and OSTD share the same version number,
which is stored in the `VERSION` file at the project root.
The current content of this file is shown below.

```
{{#include ../../../VERSION}}
```

Similarly,
the Docker images’ version number is stored in the `DOCKER_IMAGE_VERSION` file,
as shown below.

```
{{#include ../../../DOCKER_IMAGE_VERSION}}
```

We use a custom format for Docker image versions: `MAJOR.MINOR.PATCH-DATE`.
The `MAJOR.MINOR.PATCH` component aligns with the target version of published crates,
while the `DATE` component allows us to introduce non-breaking updates to the Docker images
without publishing a new crate version.
Normally,
the version in `VERSION` and the version in `DOCKER_IMAGE_VERSION`
(ignoring the `DATE` part) are identical,
except during the version bump process.

## How to Bump Versions

When preparing a new Docker image and/or a new crate release,
you must update the corresponding version numbers.

This version bump process consists of three steps.
If you only need to bump the Docker image version, follow step 1 and 2.
To publish updated crates along with the new Docker images,
complete all three steps.

A convenient utility script, `tools/bump_version.sh`,
will assist you throughout the process.

### Step 1: Submit a "Bump the Docker image version" PR

After updating the Docker image content
(specified by the `tools/docker/Dockerfile.jinja` file),
increment the Docker image version using the following command:

```
bump_version.sh --docker_version_file [major | minor | patch | date]
```

The second argument specifies which part of the Docker image version to increment.
Use `date` for non-breaking Docker image changes.
If the changes affect the published crates,
select `major`, `minor`, or `patch` in line with semantic versioning.

This command updates the `DOCKER_IMAGE_VERSION` file.
Submit these changes as a pull request.
Once merged, the CI will automatically trigger the creation of new Docker images.

### Step 2: Submit a "Switch to a new Docker image" PR

Creating new Docker images can be time-consuming.
Once the images have been pushed to DockerHub,
submit a follow-up pull request to
update all Docker image version references across the codebase.

```
bump_version.sh --docker_version_refs
```

If the new Docker image requires accompanying code changes,
include them in the same pull request to
switch to the new development environment _atomically_.

### Step 3: Submit a "Bump the project version" PR

If your changes are limited to non-breaking Docker image updates,
you are finished.
Otherwise, synchronize the version number in `VERSION` with
that in `DOCKER_IMAGE_VERSION` by running:

```
bump_version.sh --version_file
```

This command also updates all version numbers
in the `Cargo.toml` files of all crates scheduled for release.
Submit these changes as a third pull request.
After merging into the `main` branch,
the CI will automatically publish the new crate versions.
