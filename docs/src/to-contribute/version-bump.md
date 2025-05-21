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

We recommend a three-commit procedure to bump versions:
1. **Commit 1 "Bump the Docker image version"** triggers the generation of a new Docker image.
2. **Commit 2 "Switch to a new Docker image"** makes the codebase use the new Docker image.
3. **Commit 3 "Bump the project version"** triggers the release of new crates.

Depending on your exact purpose,
you may complete the version bump process with at most three commits within two PRs.
* **To make non-breaking changes to the Docker images**,
submit Commit 1 in a PR, then Commit 2 in another.
* **To make breaking changes to the Docker images and the crates' APIs**,
submit Commit 1 in a PR, then Commit 2 and 3 in another.

Across the three commits,
you will be assisted with a convenient utility script, `tools/bump_version.sh`,

### Commit 1: "Bump the Docker image version"

After updating the Docker image content,
increment the Docker image version using the following command:

```
./bump_version.sh --docker_version_file [major | minor | patch | date]
```

The second argument specifies which part of the Docker image version to increment.
Use `date` for non-breaking Docker image changes.
If the changes affect the crates intended to publish,
select `major`, `minor`, or `patch` in line with semantic versioning.

This command updates the `DOCKER_IMAGE_VERSION` file.
Submit these changes as a pull request.
Once merged, the CI will automatically trigger the creation of new Docker images.

### Commit 2: "Switch to a new Docker image"

Creating new Docker images can be time-consuming.
Once the images have been pushed to DockerHub,
write a follow-up commit to
update all Docker image version references across the codebase.

```
./bump_version.sh --docker_version_refs
```

If your purpose is to publish non-breaking changes to the Docker images,
then submit this commit in a PR and then your job is finished.
Otherwise, go on with Commit 3.

### Commit 3: "Bump the project version"

In this commit,
synchronize the version number in `VERSION` with
that in `DOCKER_IMAGE_VERSION` by running:

```
./bump_version.sh --version_file
```

This command also updates all version numbers
in the `Cargo.toml` files of all crates scheduled for release.
Pack these changes into a third commit and
submit the last two commits in a single PR.
After the PR is merged into the `main` branch,
the CI will automatically publish the new crate versions.
