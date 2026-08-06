# Using Asterinas as a Confidential Containers Guest Kernel

This guide explains how to use Asterinas
as the guest kernel for Confidential Containers (CoCo).

[Confidential Containers](https://github.com/confidential-containers/confidential-containers)
is a VM-based containers stack built on [Kata Containers](https://github.com/kata-containers/kata-containers)
and protected with [confidential computing](https://en.wikipedia.org/wiki/Confidential_computing) technology.
It runs pods inside confidential virtual machines (CVMs)
while integrating with Kubernetes and confidential computing workflows.
If you only need VM-based isolation without confidential computing,
see [Using Asterinas as a Kata Guest Kernel](kata.md) instead.

This guide uses the
[Asterinas fork of Confidential Containers](https://github.com/asterinas/confidential-containers),
which carries Asterinas-specific patches, helper scripts,
and configuration for building, installing, and testing CoCo
with Asterinas as the guest kernel.

The current Asterinas-based CoCo setup supports two paths:

- a regular CoCo runtime for local development and debugging
- a [Intel TDX](../intel-tdx.md)-powered CoCo runtime for CVMs

## Step 1: Prepare the host kernel

CoCo requires a host with KVM and vhost support.
The commands in this guide are currently written for x86-64 hosts.

Verify that the required device nodes exist:

```bash
ls /dev/kvm /dev/vhost-vsock
```

If any of them are missing,
load the matching kernel modules:

```bash
sudo modprobe kvm
sudo modprobe kvm_intel  # Or use kvm_amd on AMD hosts.
sudo modprobe vhost_vsock
```

Then make sure the user running Docker can access
`/dev/kvm` and `/dev/vhost-vsock`.

If you plan to use the TDX-powered CoCo runtime,
your host additionally requires Intel TDX hardware support,
a TDX-capable host kernel, and the TDX firmware modules.
See [Intel TDX](../intel-tdx.md) for the full list of TDX prerequisites.

## Step 2: Enter the CoCo environment

We provide the `asterinas/coco` image with CoCo and Asterinas preinstalled.
End users should use it directly.
Kernel developers should additionally mount the local Asterinas source.

### Step 2.1: Prepare Docker arguments

Define the following Docker arguments:

```bash
COCO_DOCKER_ARGS=(
    --cgroupns host
    --privileged
    --device /dev/kvm
    --device /dev/vhost-vsock
    --tmpfs /var/lib/containerd-nydus:rw,size=512m
)
```

These flags are required for CoCo:

- `--cgroupns host` shares the host cgroup namespace
  so that `containerd` inside the container can manage CoCo workloads.
- `--privileged` is required for KVM and nested container management.
- `--device /dev/kvm` and `--device /dev/vhost-vsock`
  expose the virtualization devices
  that CoCo needs.
- `--tmpfs /var/lib/containerd-nydus:rw,size=512m`
  gives the nydus image service enough temporary storage space for guest-side image pulling.

### Step 2.2: Start the Docker container

#### For end users

Run an `asterinas/coco` Docker container
to enter an environment with CoCo and Asterinas preinstalled:

```bash
docker run -it \
    "${COCO_DOCKER_ARGS[@]}" \
    asterinas/coco:0.18.0-20260603
```

The image bundles the CoCo runtime, the Asterinas guest kernel,
the cluster setup script `/opt/coco/setup-coco-k8s.sh`,
and the pod manifests under `/opt/coco/manifests/`
used in the steps below.

After entering the container,
continue with [Step 3](#step-3-start-a-coco-workload).

#### For kernel developers

Clone the Asterinas source,
then run an `asterinas/coco` Docker container
with the source tree mounted:

```bash
# Assumes the Asterinas source has already been cloned locally.
ASTERINAS_SRC=$HOME/asterinas

docker run -it \
    "${COCO_DOCKER_ARGS[@]}" \
    -v "${ASTERINAS_SRC}:/root/asterinas" \
    asterinas/coco:0.18.0-20260603
```

The `asterinas/coco` image is built on top of the `asterinas/asterinas` image,
so it already includes all the dependencies needed to build Asterinas.
This means kernel developers can build Asterinas directly inside the CoCo image
by simply mounting the local Asterinas source tree.

After entering the container,
continue with [Step 3](#step-3-start-a-coco-workload).

## Step 3: Start a CoCo workload

### Step 3.1: Set up the cluster

Set up the local Kubernetes cluster with:

```bash
/opt/coco/setup-coco-k8s.sh
```

This script prepares the Kubernetes control plane
and configures the CoCo runtime within it,
including `containerd`, CNI,
and the CoCo runtime configuration.

### Step 3.2: Run a CoCo workload

CoCo manages workloads through Kubernetes,
where you submit a pod manifest to the cluster to run a workload.

Apply the regular CoCo workload bundled in the CoCo Docker image:

```bash
kubectl apply -f /opt/coco/manifests/alpine-kata-qemu-coco-dev.yaml
```

**Or** the TDX CoCo workload (requires Intel TDX hardware support on the host):

```bash
kubectl apply -f /opt/coco/manifests/alpine-kata-qemu-tdx.yaml
```

After applying the manifest,
wait for the pod to become ready:

```bash
kubectl wait --for=condition=Ready pod/alpine-coco --timeout=2m
```

The regular manifest bundled looks like:

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: alpine-coco
spec:
  runtimeClassName: kata-qemu-coco-dev-asterinas
  containers:
    - name: alpine
      image: docker.io/library/alpine:3.22
      command: ["sleep", "infinity"]
```

This manifest runs an Alpine container as pod `alpine-coco`,
with `sleep infinity` to keep it running for `kubectl exec` verification.

The `runtimeClassName` field determines which CoCo runtime the pod uses.
The Asterinas-based CoCo supports two runtime classes:

- `kata-qemu-coco-dev-asterinas` — for regular CoCo
- `kata-qemu-tdx-asterinas` — for TDX-backed CVMs (requires Intel TDX hardware support)

### Step 3.3: Verify the guest

After the pod becomes ready,
run the following commands to check the guest kernel and the workload rootfs:

```bash
kubectl exec -it alpine-coco -- /bin/sh

# Inside the container
cat /proc/cmdline
cat /etc/alpine-release
```

The `/proc/cmdline` output should contain the Asterinas kernel image path.
Look for the keyword:

```text
aster-kernel-osdk-bin
```

This is the most direct check that CoCo booted an Asterinas guest kernel.
The `/etc/alpine-release` output should print the Alpine version
of the container rootfs.

### Step 3.4: Clean up

After verification,
remove the pod with:

```bash
kubectl delete pod alpine-coco
```

Both bundled manifests create a pod named `alpine-coco`,
so only one can be applied at a time.
To switch to the other runtime class,
delete the existing pod first by running the command above.

## Step 4: Use a local kernel (optional)

This step is for the kernel-developer setup only: it requires the Asterinas source tree,
so it assumes you entered the CoCo environment through the
[For kernel developers](#for-kernel-developers) setup in Step 2.2.

Build the kernel and verify the output images:

```bash
# Build the regular guest kernel
cd /root/asterinas && make kernel BOOT_METHOD=direct-elf

# Verify the regular build output
ls /root/asterinas/target/osdk/aster-kernel-osdk-bin.elf

# Build the TDX guest kernel (only needed for the CVM path)
cd /root/asterinas && make kernel BOOT_METHOD=direct-elf INTEL_TDX=1

# Verify the TDX build output
# (Note that the TDX kernel uses a bzImage format without the `.elf` extension.)
ls /root/asterinas/target/osdk/aster-kernel-osdk-bin
```

Then adjust the CoCo runtime configuration to point to the locally built Asterinas kernel.

For the regular runtime, edit:

- `/opt/coco/config/configuration-qemu-coco-dev-asterinas.toml`

and set `kernel` under `[hypervisor.qemu]`:

```toml
[hypervisor.qemu]
kernel = "/root/asterinas/target/osdk/aster-kernel-osdk-bin.elf"
```

For the TDX runtime, edit:

- `/opt/coco/config/configuration-qemu-tdx-asterinas.toml`

and set `kernel` under `[hypervisor.qemu]`:

```toml
[hypervisor.qemu]
kernel = "/root/asterinas/target/osdk/aster-kernel-osdk-bin"
```

The configuration takes effect immediately — no service restart is needed.
Follow [Step 3.2](#step-32-run-a-coco-workload) to verify
that the guest boots with your locally compiled Asterinas kernel.
