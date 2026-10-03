# The Asterinas Book

<p align="center">
    <img src="images/logo_en.svg" alt="asterinas-logo" width="620"><br>
</p>

Welcome to the documentation for Asterinas,
an open-source project and community
focused on developing cutting-edge Rust OS kernels.

## Book Structure

This book is divided into six distinct parts:

#### [Part 1: Asterinas NixOS](distro/)

Asterinas NixOS is the first distribution built on top of the Asterinas kernel.
It is based on NixOS,
leveraging its powerful configuration model
and rich package ecosystem,
while swapping out the Linux kernel for Asterinas.

#### [Part 2: Asterinas Kernel](kernel/)

Explore the modern OS kernel at the heart of Asterinas.
Designed to realize the full potential of Rust,
Asterinas Kernel implements the Linux ABI in a safe and efficient way.
This means it can seamlessly replace Linux,
offering enhanced safety and security.

#### [Part 3: Asterinas OSTD](ostd/)

The Asterinas OSTD lays down a minimalistic, powerful, and solid foundation
for OS development.
It's akin to Rust's `std` crate
but crafted for the demands of _safe_ Rust OS development.
The Asterinas Kernel is built on this very OSTD.

#### [Part 4: Asterinas OSDK](osdk/guide/)

The OSDK is a command-line tool
that streamlines the workflow to
create, build, test, and run Rust OS projects
that are built upon Asterinas OSTD.
Developed specifically for OS developers,
it extends Rust's Cargo tool to better suite their specific needs.
OSDK is instrumental in the development of Asterinas Kernel.

#### [Part 5: Contributing to Asterinas](to-contribute/)

Asterinas is in its early stage
and welcomes your contributions!
This part provides guidance
on how you can become an integral part of the Asterinas project.

#### [Part 6: Requests for Comments (RFCs)](rfcs/)

Significant decisions in Asterinas are made through a transparent RFC process.
This part describes the RFC process
and archives all approvaed RFCs.

## Publications

Asterinas has been the subject of the following publications,
listed from newest to oldest:

* [_RusyFuzz: Unhandled Exception Guided Fuzzing for Rust OS Kernel_](https://conf.researchr.org/details/icse-2026/icse-2026-research-track/96/RusyFuzz-Unhandled-Exception-Guided-Fuzzing-for-Rust-OS-Kernel),
  **ICSE 2026**.
* [_MlsDisk: Trusted Block Storage for TEEs Based on Layered Secure Logging_](https://www.usenix.org/conference/fast26/presentation/xu),
  **FAST 2026**.
* [_CortenMM: Efficient Memory Management with Strong Correctness Guarantees_](https://dl.acm.org/doi/10.1145/3731569.3764836),
  **SOSP 2025**, Best Paper Award.
* [_Asterinas: A Linux ABI-Compatible, Rust-Based Framekernel OS with a Small and Sound TCB_](https://www.usenix.org/conference/atc25/presentation/peng-yuke),
  **USENIX ATC 2025**.
* [_Converos: Practical Model Checking for Verifying Rust OS Kernel Concurrency_](https://www.usenix.org/conference/atc25/presentation/tang),
  **USENIX ATC 2025**.
* [_Asterinas: A Rust-Based Framekernel to Reimagine Linux in the 2020s_](https://www.usenix.org/publications/loginonline/asterinas-rust-based-framekernel-reimagine-linux-2020s),
  **USENIX _;login:_ 2025**.

## Licensing

Asterinas's source code and documentation primarily use the
[Mozilla Public License (MPL), Version 2.0](https://github.com/asterinas/asterinas/blob/main/LICENSE-MPL).
Select components are under more permissive licenses,
detailed [here](https://github.com/asterinas/asterinas/blob/main/.licenserc.yaml).

Our choice of the [weak-copyleft](https://www.tldrlegal.com/license/mozilla-public-license-2-0-mpl-2) MPL license reflects a strategic balance:

1. **Commitment to open-source freedom**:
We believe that OS kernels are a communal asset that should benefit humanity.
The MPL ensures that any alterations to MPL-covered files remain open source,
aligning with our vision.
Additionally, we do not require contributors
to sign a Contributor License Agreement (CLA),
[preserving their rights and preventing the possibility of their contributions being made closed source](https://drewdevault.com/2018/10/05/Dont-sign-a-CLA.html).

2. **Accommodating proprietary modules**:
Recognizing the evolving landscape
where large corporations also contribute significantly to open-source,
we accommodate the business need for proprietary kernel modules.
Unlike GPL,
the MPL permits the linking of MPL-covered files with proprietary code.

In conclusion, we believe that
MPL is the best choice
to foster a vibrant, robust, and inclusive open-source community around Asterinas.
