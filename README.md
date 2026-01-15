<p align="center">
    <img src="book/src/images/logo_en.svg" alt="asterinas-logo" width="620"><br>
    A secure, fast, and general-purpose OS kernel written in Rust and compatible with Linux<br/>
</p>

<!-- Asterinas NixOS 0.17.0 demo. It is uploaded as a Github attachment
so that GitHub will render that URL as a video player in Markdown.
The original file name will be displayed up in the top bar of the video player.
So make sure you give the video file a cool name before uploading it.
-->
https://github.com/user-attachments/assets/26be2d18-994d-4658-a1b8-f8959bd88b75

<p align="center">
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_x86.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_x86.yml/badge.svg?event=push" alt="Test x86-64" style="max-width: 100%;"></a>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_riscv.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_riscv.yml/badge.svg?event=push" alt="Test riscv64" style="max-width: 100%;"></a>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_loongarch.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_loongarch.yml/badge.svg?event=push" alt="Test loongarch64" style="max-width: 100%;"></a>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_x86_tdx.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_x86_tdx.yml/badge.svg" alt="Test Intel TDX" style="max-width: 100%;"></a>
    <a href="https://asterinas.github.io/benchmark/x86-64/"><img src="https://github.com/asterinas/asterinas/actions/workflows/benchmark_x86.yml/badge.svg" alt="Benchmark x86-64" style="max-width: 100%;"></a>
    <a href="https://asterinas.github.io/benchmark/tdx/"><img src="https://github.com/asterinas/asterinas/actions/workflows/benchmark_x86_tdx.yml/badge.svg" alt="Benchmark Intel TDX" style="max-width: 100%;"></a>
    <br/>
</p>

**News:**
* 2025-12-08: **FAST 2026** accepted a paper on a novel secure storage solution having been integrated into Asterinas: _MlsDisk: Trusted Block Storage for TEEs Based on Layered Secure Logging_.
* 2025-10-17: **ICSE 2026** accepted yet another paper about Asterinas: _RusyFuzz: Unhandled Exception Guided Fuzzing for Rust OS Kernel_.
* 2025-10-14: [*CortenMM: Efficient Memory Management with Strong Correctness Guarantees*](https://dl.acm.org/doi/10.1145/3731569.3764836) received the **Best Paper Award** at **SOSP 2025**.
* 2025-07-23: **SOSP 2025** accepted another Asterinas paper: [*CortenMM: Efficient Memory Management with Strong Correctness Guarantees*](https://dl.acm.org/doi/10.1145/3731569.3764836).
* 2025-06-18: **USENIX _;login:_ magazine** published [*Asterinas: A Rust-Based Framekernel to Reimagine Linux in the 2020s*](https://www.usenix.org/publications/loginonline/asterinas-rust-based-framekernel-reimagine-linux-2020s).
* 2025-04-30: **USENIX ATC 2025** accepted two Asterinas papers:
    1. [*Asterinas: A Linux ABI-Compatible, Rust-Based Framekernel OS with a Small and Sound TCB*](https://www.usenix.org/conference/atc25/presentation/peng-yuke);
    2. [*Converos: Practical Model Checking for Verifying Rust OS Kernel Concurrency*](https://www.usenix.org/conference/atc25/presentation/tang).

Congratulations to the Asterinas community🎉🎉🎉

## Introducing Asterinas

Asterinas is a _secure_, _fast_, and _general-purpose_ OS kernel
that provides _Linux-compatible_ ABI.
It can serve as a seamless replacement for Linux
while enhancing _memory safety_ and _developer friendliness_.

* Asterinas prioritizes memory safety
by employing Rust as its sole programming language
and limiting the use of _unsafe Rust_
to a clearly defined and minimal Trusted Computing Base (TCB).
This innovative approach,
known as [the framekernel architecture](https://asterinas.github.io/book/kernel/the-framekernel-architecture.html),
establishes Asterinas as a more secure and dependable kernel option.

* Asterinas surpasses Linux in terms of developer friendliness.
It empowers kernel developers to
(1) utilize the more productive Rust programming language,
(2) leverage a purpose-built toolkit called [OSDK](https://asterinas.github.io/book/osdk/guide/index.html) to streamline their workflows,
and (3) choose between releasing their kernel modules as open source
or keeping them proprietary,
thanks to the flexibility offered by [MPL](#License).

While the journey towards a production-grade OS kernel is challenging,
we are steadfastly progressing towards this goal.
Over the course of 2024,
we significantly enhanced Asterinas's maturity,
as detailed in [our end-year report](https://asterinas.github.io/2025/01/20/asterinas-in-2024.html).
In December 2025,
we launched [Asterinas NixOS](https://asterinas.github.io/book/distro/index.html),
an Asterinas distribution based on [NixOS](https://nixos.org/).
This release marks Asterinas reaching the Minimum Viable Product (MVP) milestone.
In 2026, we aim to make Asterinas production-ready on x86-64 virtual machines and attract real users.

## Getting Started

### For End Users

We provide [Asterinas NixOS ISO Installer](https://github.com/asterinas/asterinas/releases)
to make the Asterinas kernel more accessible for early adopters and enthusiasts.
We encourage you to try out Asterinas NixOS and share feedback.
Instructions on how to use the ISO installer can be found [here](https://asterinas.github.io/book/distro/index.html#end-users).

**Disclaimer: Asterinas is an independent, community-led project.
Asterinas NixOS is _not_ an official NixOS project and has _no_ affiliation with the NixOS Foundation. _No_ sponsorship or endorsement is implied.**

### For Kernel Developers

Follow the steps below to get Asterinas up and running.

1. Download the latest source code on an x86-64 Linux machine:

    ```bash
    git clone https://github.com/asterinas/asterinas
    ```

2. Run a Docker container as the development environment:

    ```bash
    docker run -it --privileged --network=host -v /dev:/dev -v $(pwd)/asterinas:/root/asterinas asterinas/asterinas:0.17.0-20260114
    ```

3. Inside the container,
go to the project folder (`/root/asterinas`) and run:

    ```bash
    make kernel
    make run_kernel
    ```

    This results in a VM running the Asterinas kernel with a small initramfs.

4. To install and test real-world applications on Asterinas,
build and run Asterinas NixOS in a VM:

    ```bash
    make nixos
    make run_nixos
    ```
    
    This boots into an interactive shell in Asterinas NixOS,
    where you can use Nix to install and try more packages.

## The Book

See [The Asterinas Book](https://asterinas.github.io/book/) to learn more about the project.

## License

Asterinas's source code and documentation primarily use the
[Mozilla Public License (MPL), Version 2.0](https://github.com/asterinas/asterinas/blob/main/LICENSE-MPL).
Select components are under more permissive licenses,
detailed [here](https://github.com/asterinas/asterinas/blob/main/.licenserc.yaml). For the rationales behind the choice of MPL, see [here](https://asterinas.github.io/book/index.html#licensing).
