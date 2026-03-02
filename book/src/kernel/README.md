# Asterinas Kernel

## Overview

Asterinas is a _secure_, _fast_, and _general-purpose_ OS kernel
that provides an _Linux-compatible_ ABI.
It can serve as a seamless replacement for Linux
while enhancing _memory safety_ and _developer friendliness_.

* Asterinas prioritizes memory safety
by employing Rust as its sole programming language
and limiting the use of _unsafe Rust_
to a clearly defined and minimal Trusted Computing Base (TCB).
This innovative approach,
known as [the framekernel architecture](the-framekernel-architecture.md),
establishes Asterinas as a more secure and dependable kernel option.

* Asterinas surpasses Linux in terms of developer friendliness.
It empowers kernel developers to
(1) utilize the more productive Rust programming language,
(2) leverage a purpose-built toolkit called [OSDK]() to streamline their workflows,
and (3) choose between releasing their kernel modules as open source
or keeping them proprietary,
thanks to the flexibility offered by [MPL](../).

While the journey towards a production-grade OS kernel can be challenging,
we are steadfastly progressing towards our goal.

## Supported CPU Architectures

Asterinas targets modern, 64-bit platforms only.

A **development platform** is where you build and test Asterinas
(i.e., the host machine running the Docker-based development environment).

| Development Platform |
| -------------------- |
| x86-64               |

A **deployment platform** is a CPU architecture
that Asterinas can run on as an OS kernel.

| Deployment Platform | Tier   |
| ------------------- | ------ |
| x86-64              | Tier 1 |
| x86-64 (Intel TDX)  | Tier 2 |
| RISC-V 64           | Tier 2 |
| LoongArch 64        | Tier 3 |

- **Tier 1:** Fully supported and tested.
  CI runs the full test suite on every PR.
- **Tier 2:** Actively developed with basic functionality working.
  CI runs build checks and basic tests on a regular basis
  (per PR for RISC-V and nightly for Intel TDX),
  but the full test suite is not yet covered.
- **Tier 3:** Early-stage or experimental.
  The kernel can boot and perform basic operations,
  but CI coverage is limited and
  may not include automated runtime tests for every pull request.

## Getting Started

Get yourself an x86-64 Linux machine with Docker installed.
Follow the three simple steps below to get Asterinas up and running.

<!-- REMINDER: Be careful when editing the first two steps
since `distro/README.md` references them -->
1. Download the latest source code.

    ```bash
    git clone https://github.com/asterinas/asterinas
    ```

2. Run a Docker container as the development environment.

    ```bash
    docker run -it --privileged \
                --network=host \
                -v /dev:/dev \
                -v $(pwd)/asterinas:/root/asterinas \
                asterinas/asterinas:0.17.0-20260227
    ```

3. Inside the container, go to the project folder to build and run Asterinas.

    ```bash
    make kernel
    make run_kernel
    ```

If everything goes well, Asterinas is now up and running inside a VM.
