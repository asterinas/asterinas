# Roadmap

Asterinas is a general-purpose OS kernel
designed to support multiple CPU architectures and a variety of use cases.
Currently, it only supports x86-64 VMs.
Our roadmap includes the following plans:

* By 2024, we aim to achieve production-ready status for VM environments on x86-64.
* In 2025 and beyond, we will expand our support for CPU architectures and hardware devices.

## Target Early Use Cases

One of the biggest challenges for a new OS kernel is driver support.
Linux has been widely accepted due to its wide range of hardware support.
As a newcomer, Asterinas faces the challenge of implementing drivers
for all devices on a target platform,
which would take a significant amount of time.

To address this obstacle,
we have decided to enter the cloud market first.
In an IaaS (Infrastructure-as-a-Service) cloud, workloads of different tenants are run in VMs
or [VM-style bare-metal servers](https://dl.acm.org/doi/10.1145/3373376.3378507)
for maximum isolation and elasticity.
The main device driver requirement for the VM environment is virtio,
which is already supported by Asterinas.
Therefore, using Asterinas as the guest OS of a VM
or the host OS of a VM-style bare-metal server in production
looks quite feasible in the near future.

Asterinas provides high assurance of memory safety
thanks to [the framekernel architecture](the-framekernel-architecture.md).
Thus, in the cloud setting,
Asterinas is attractive for usage scenarios
where Linux ABI is necessary but Linux itself is considered insecure
due to its large Trusted Computing Base (TCB) and memory unsafety.
Specifically, we are focusing on two use cases:

1. VM-based TEEs:
All major CPU architectures have introduced
VM-based Trusted Execution Environment (TEE) technology,
such as ARM CCA, AMD SEV, and Intel TDX.
Applications running inside TEEs often handle private or sensitive data.
By running on a lightweight and memory-safe OS kernel like Asterinas,
they can greatly enhance security and privacy.

2. Secure containers:
In the cloud-native era, applications are commonly deployed in containers.
The popular container runtimes like runc and Docker rely on
the OS-level isolation enforced by Linux.
However, [Linux containers are prone to privilege escalation bugs](https://dl.acm.org/doi/10.1145/3274694.3274720).
With its safety and security prioritized architecture,
Asterinas can offer more reliable OS-level isolation,
making it ideal for secure containers.
