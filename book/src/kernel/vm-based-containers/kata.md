# Using Asterinas as a Kata Guest Kernel

This guide explains how to use Asterinas
as the guest kernel for Kata Containers.

[Kata Containers](https://github.com/kata-containers/kata-containers)
is a VM-based container runtime.
It runs each sandbox or standalone container workload
inside a lightweight virtual machine,
so workloads do not share the host kernel.
In Kubernetes, the sandbox maps to a pod,
so containers in the same pod can share that VM.
This design combines the deployment model of containers
with the stronger isolation boundary of virtual machines.
It reduces the risk that a compromised workload can affect
the host or other containers.

This guide uses the
[Asterinas fork of Kata Containers](https://github.com/asterinas/kata-containers),
which carries Asterinas-specific patches, helper scripts,
and configuration for building, installing, and testing Kata
with Asterinas as the guest kernel.

## Step 1: Prepare the host kernel

Kata Containers requires a host with KVM and vhost support.
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

## Step 2: Enter the Kata-ready environment

Use one of the Docker images below to enter an environment
that can run Asterinas-powered Kata containers.
End users should use the prebuilt `asterinas/kata` image.
Kernel developers should use the `asterinas/asterinas` image
with source mounts.

### Step 2.1: Prepare Docker arguments

Define the following Docker arguments:

```bash
KATA_DOCKER_ARGS=(
    --cgroupns host
    --privileged
    --device /dev/kvm
    --device /dev/vhost-vsock
    --tmpfs /tmp:exec,mode=1777,size=8g
    --tmpfs /var/lib/containerd:exec,mode=755,size=8g
)
```

These flags are required for Kata:

- `--cgroupns host` shares the host cgroup namespace
  so that `containerd` inside the container can manage Kata workloads.
- `--privileged` is required for KVM and nested container management.
- `--device /dev/kvm` and `--device /dev/vhost-vsock`
  expose the virtualization devices
  that Kata needs.
- `--tmpfs /tmp:exec,...` and `--tmpfs /var/lib/containerd:exec,...`
  give Kata enough temporary storage space to create containers.

These Docker arguments apply to both end users and kernel developers.

### Step 2.2: Start the Docker container

#### For end users

Run an `asterinas/kata` Docker container
to enter an environment with Kata and Asterinas preinstalled:

```bash
docker run -it \
    "${KATA_DOCKER_ARGS[@]}" \
    -w /root/kata-containers \
    asterinas/kata:0.17.2-20260407
```

The Docker image includes Kata, the Asterinas guest kernel,
the Kata configuration,
and the `tools/kata/` helper scripts in `/root/kata-containers`.
The command above makes `/root/kata-containers` the working directory.

After entering the container,
continue with [Step 3: Start a Kata workload](#step-3-start-a-kata-workload).

#### For kernel developers

Clone the Asterinas fork of Kata Containers,
then run an `asterinas/asterinas` Docker container
with both source trees mounted:

```bash
# Assumes the Asterinas source has already been cloned locally.
ASTERINAS_SRC=$HOME/asterinas

# Clones the Asterinas fork of kata-containers locally as well.
cd $HOME && git clone https://github.com/asterinas/kata-containers.git
KATA_SRC=$HOME/kata-containers

docker run -it \
    "${KATA_DOCKER_ARGS[@]}" \
    -v "${ASTERINAS_SRC}:/root/asterinas" \
    -v "${KATA_SRC}:/root/kata-containers" \
    -w /root/kata-containers \
    asterinas/asterinas:0.17.2-20260407
```

This setup lets you rebuild the kernel
and test how kernel changes affect Kata workloads.
The Asterinas source tree is mounted at `/root/asterinas`.
The Asterinas fork of Kata Containers is mounted at `/root/kata-containers`.
Use the Asterinas fork for changes to the Asterinas guest-kernel integration
until those patches, scripts, and configuration are upstreamed.
The command above makes `/root/kata-containers` the working directory,
so the `tools/kata/` helper scripts are available
at the relative paths used below.

Install the Kata dependencies and configuration
after entering the `asterinas/asterinas` image:

```bash
tools/kata/kata_env.sh install
```

Then continue with [Step 3: Start a Kata workload](#step-3-start-a-kata-workload).

## Step 3: Start a Kata workload

### Step 3.1: Start services

Start `containerd` and `syslogd`,
the background services required by Kata:

```bash
tools/kata/kata_services.sh start
```

Check their status to make sure they started successfully:

```bash
tools/kata/kata_services.sh status
```

### Step 3.2: Run Alpine with Kata

Use `nerdctl` with Kata to start an Alpine container:

```bash
nerdctl run \
    --cgroup-manager cgroupfs \
    --net none \
    --runtime io.containerd.kata.v2 \
    --name foo \
    -it \
    docker.io/alpine:latest
```

The `nerdctl` flags above are also required:

- `--cgroup-manager cgroupfs` is required because the example runs
  inside a Docker container where the systemd cgroup driver
  is not available.
- `--net none` is required because Asterinas does not yet support
  hot-plugged network devices,
  so workloads inside the guest cannot access the network.

### Step 3.3: Verify the guest

Run the following commands after the container starts.
The first checks the guest kernel,
and the second checks the workload rootfs:

```bash
cat /proc/cmdline
cat /etc/alpine-release
```

The `/proc/cmdline` output should contain the Asterinas kernel image path.
For the prebuilt `asterinas/kata` image,
look for:

```text
/opt/kata/share/kata-containers/aster-kernel-osdk-bin.qemu_elf
```

For a locally built kernel,
look for:

```text
/root/asterinas/target/osdk/aster-kernel-osdk-bin.qemu_elf
```

This is the most direct check that Kata booted an Asterinas guest kernel.
The `/etc/alpine-release` output should print the Alpine version
of the container rootfs.

### Step 3.4: Clean up

Remove the container after exiting it:

```bash
nerdctl rm foo
```

## Step 4: Use a local kernel (optional)

This step is for the kernel-developer setup only:
it requires the Asterinas source tree,
so it assumes you entered the Kata-ready environment through the
[For kernel developers](#for-kernel-developers) setup in Step 2.2.
It is not available from the prebuilt `asterinas/kata` image.

Build the kernel first:

```bash
(cd /root/asterinas && make kernel BOOT_METHOD=qemu-direct)
```

Verify that the command produced the kernel image:

```bash
ls /root/asterinas/target/osdk/aster-kernel-osdk-bin.qemu_elf
```

Pass that path to `tools/kata/kata_env.sh install`:

```bash
tools/kata/kata_env.sh install \
    --kernel /root/asterinas/target/osdk/aster-kernel-osdk-bin.qemu_elf
```

Configuring a local kernel re-runs `tools/kata/kata_env.sh install`
(re-applying the Kata install)
and writes a new `/etc/kata-containers/configuration.toml`
that points at your local kernel.

Start or restart the Kata services.
If the services are already running from the previous run,
stop them first:

```bash
tools/kata/kata_services.sh stop
```

Start the services again:

```bash
tools/kata/kata_services.sh start
tools/kata/kata_services.sh status
```

Then run the `nerdctl run` command from
[Step 3.2: Run Alpine with Kata](#step-32-run-alpine-with-kata).
Kata will boot the workload with your local Asterinas kernel.
