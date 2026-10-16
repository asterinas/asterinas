#!/bin/bash
# SPDX-License-Identifier: MPL-2.0

set -euxo pipefail
export LC_ALL=C
logs="$RUNNER_TEMP/docker-images"
mkdir -p "$logs"
source_sha=$(git rev-parse HEAD)
tag="ci-${source_sha:0:12}-$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT-$(uname -m)"
rust_version=$(python3 -c 'import tomllib; print(tomllib.load(open("rust-toolchain.toml", "rb"))["toolchain"]["channel"])')
nixpkgs_url=$(python3 - <<'PY'
import json
lock = json.load(open("flake.lock"))
source = lock["nodes"][lock["nodes"][lock["root"]]["inputs"]["nixpkgs"]]["locked"]
print(f"https://github.com/{source['owner']}/{source['repo']}/archive/{source['rev']}.tar.gz")
PY
)
case "$(uname -m)" in
    x86_64) architecture=amd64; enable_kvm=1; devices=(--device /dev/kvm)
        test -c /dev/kvm ;;
    aarch64) architecture=arm64; enable_kvm=0; devices=() ;;
    *) exit 1 ;;
esac
{
    git rev-parse HEAD 'HEAD^{tree}'
    sha256sum flake.lock
    uname -a
    docker version
    docker buildx version
    printf 'Workflow: %s\nRunner image: %s %s\nRust: %s\n' \
        "$GITHUB_WORKFLOW_SHA" "${ImageOS:-unknown}" "${ImageVersion:-unknown}" "$rust_version"
} > "$logs/identity.txt"
sha256sum flake.lock > "$logs/lock.sha256"
df -h
parent=
for image in osdk-dev prebuilt-nix-packages kernel-dev dev; do
    case "$image" in
        osdk-dev) dockerfile=osdk/tools/docker/Dockerfile ;;
        prebuilt-nix-packages) dockerfile=tools/dev_env/docker/prebuilt-nix-packages/Dockerfile ;;
        kernel-dev) dockerfile=tools/dev_env/docker/kernel-dev/Dockerfile ;;
        dev) dockerfile=tools/dev_env/docker/Dockerfile ;;
    esac
    if [[ -n "$parent" ]]; then
        test "$(docker image inspect --format '{{.Id}}' "asterinas/$parent:$tag")" = "$(cat "$logs/$parent.id")"
    fi
    # The default Docker driver lets each stage consume the previous local image.
    docker buildx build --builder default --load --pull=false --progress=plain \
        --platform "linux/$architecture" --build-arg "BASE_VERSION=$tag" \
        --build-arg "ASTER_RUST_VERSION=$rust_version" --iidfile "$logs/$image.id" \
        --tag "asterinas/$image:$tag" --file "$dockerfile" . 2>&1 | tee "$logs/$image-build.log"
    image_id=$(cat "$logs/$image.id")
    docker image inspect "$image_id" > "$logs/$image.inspect.json"
    test "$(docker image inspect --format '{{.Architecture}}' "$image_id")" = "$architecture"
    if [[ -n "$parent" ]]; then
        jq -e --slurpfile parent "$logs/$parent.inspect.json" \
            '.[0].RootFS.Layers[:($parent[0][0].RootFS.Layers | length)] == $parent[0][0].RootFS.Layers' \
            "$logs/$image.inspect.json"
    fi
    docker run --rm --pull=never --network none --entrypoint nix "$image_id" \
        --extra-experimental-features nix-command path-info --all | sort -u > "$logs/$image.paths"
    if [[ "$image" == osdk-dev ]]; then
        docker run --rm --pull=never --mount "type=bind,src=$PWD,dst=/source,readonly" \
            --entrypoint nix "$image_id" --extra-experimental-features 'nix-command flakes' \
            eval --json --accept-flake-config --no-update-lock-file \
            --apply 'p: { qemu = p.qemu.outPath; version = p.qemu.version; vdso = p.vdso.outPath; }' \
            "path:/source#packages.$(uname -m)-linux" > "$logs/outputs.json"
        qemu=$(jq -r .qemu "$logs/outputs.json")
        qemu_version=$(jq -r .version "$logs/outputs.json")
        vdso=$(jq -r .vdso "$logs/outputs.json")
    fi
    if [[ "$image" == kernel-dev || "$image" == dev ]]; then
        comm -23 "$logs/prebuilt-nix-packages.paths" "$logs/$image.paths" > "$logs/$image.missing"
        test ! -s "$logs/$image.missing"
    fi
    docker run --rm --pull=never -i --network none --entrypoint bash "$image_id" \
        --noprofile --norc -s -- "$qemu" "$qemu_version" "$vdso" "$image" no "$nixpkgs_url" \
        < tools/github_workflows/check_docker_image.sh 2>&1 | tee "$logs/$image-runtime.log"
    if [[ "$image" == osdk-dev || "$image" == dev ]]; then
        # GC tests never modify the images inherited by the next build stage.
        docker run --rm --pull=never -i --network none --entrypoint bash "$image_id" \
            --noprofile --norc -s -- "$qemu" "$qemu_version" "$vdso" "$image" yes "$nixpkgs_url" \
            < tools/github_workflows/check_docker_image.sh 2>&1 | tee "$logs/$image-gc.log"
    fi
    parent=$image
    df -h
done

docker run --rm --pull=never --entrypoint nix-shell "$(cat "$logs/dev.id")" \
    -p hello --run hello 2>&1 | tee "$logs/channel.log"

# Use a fresh checkout on CI. Build logs live outside the Docker context.
container="docker-image-tests-$tag"
trap 'docker rm -f "$container" >/dev/null 2>&1 || true' EXIT
# The OSDK debugging tests need an x86-capable GDB, which ARM64 images lack.
if [[ "$architecture" == amd64 ]]; then
    docker run -d --pull=never --name "$container" "${devices[@]}" \
        --mount "type=bind,src=$PWD,dst=/root/asterinas" --workdir /root/asterinas \
        --entrypoint bash "$(cat "$logs/osdk-dev.id")" --noprofile --norc -c 'sleep infinity'
    docker exec "$container" git config --global --add safe.directory /root/asterinas
    docker exec "$container" timeout 30m make test_osdk 2>&1 | tee "$logs/osdk-test.log"
    docker rm -f "$container"
else
    echo "Full OSDK tests are not covered on ARM64: the native GDB cannot debug x86_64." \
        | tee "$logs/osdk-test.log"
fi
docker run -d --pull=never --name "$container" "${devices[@]}" \
    --mount "type=bind,src=$PWD,dst=/root/asterinas" --workdir /root/asterinas \
    --entrypoint bash "$(cat "$logs/kernel-dev.id")" --noprofile --norc -c 'sleep infinity'
docker exec "$container" git config --global --add safe.directory /root/asterinas
if [[ "$enable_kvm" == 1 ]]; then
    docker exec "$container" bash -euc 'test -r /dev/kvm && test -w /dev/kvm'
fi
docker exec "$container" timeout 30m make check 2>&1 | tee "$logs/check.log"
docker exec "$container" timeout 30m make test 2>&1 | tee "$logs/test.log"
rm -f qemu.log
docker exec "$container" timeout 30m make run_kernel AUTO_TEST=boot TARGET_ARCH=x86_64 \
    NETDEV=user "ENABLE_KVM=$enable_kvm" 2>&1 | tee "$logs/boot.log"
cp qemu.log "$logs/qemu.log"
cp target/osdk/asterinas/bundle.toml "$logs/boot-bundle.toml"
grep '^Successfully booted\.' "$logs/qemu.log"
if [[ "$enable_kvm" == 1 ]]; then
    grep -F -- '-accel kvm' "$logs/boot-bundle.toml"
fi
sha256sum --check "$logs/lock.sha256"
echo "Candidate image validation passed."
