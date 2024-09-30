# The Asterinas Book

<p align="center">
    <img src="images/logo_en.svg" alt="asterinas-logo" width="620"><br>
</p>

Welcome to the documentation for Asterinas,
an open-source project and community
focused on developing cutting-edge Rust OS kernels.

## Book Structure

This book is divided into five distinct parts:

#### [Part 1: Asterinas Kernel](kernel/)

Explore the modern OS kernel at the heart of Asterinas.
Designed to realize the full potential of Rust,
Asterinas Kernel implements the Linux ABI in a safe and efficient way.
This means it can seamlessly replace Linux,
offering enhanced safety and security.

#### [Part 2: Asterinas OSTD](ostd/)

The Asterinas OSTD lays down a minimalistic, powerful, and solid foundation
for OS development.
It's akin to Rust's `std` crate
but crafted for the demands of _safe_ Rust OS development.
The Asterinas Kernel is built on this very OSTD.

#### [Part 3: Asterinas OSDK](osdk/guide/)

The OSDK is a command-line tool
that streamlines the workflow to 
create, build, test, and run Rust OS projects
that are built upon Asterinas OSTD.
Developed specifically for OS developers,
it extends Rust's Cargo tool to better suite their specific needs.
OSDK is instrumental in the development of Asterinas Kernel.

#### [Part 4: Contributing to Asterinas](to-contribute/)

Asterinas is in its early stage
and welcomes your contributions!
This part provides guidance
on how you can become an integral part of the Asterinas project.

#### [Part 5: Requests for Comments (RFCs)](rfcs/)

Significant decisions in Asterinas are made through a transparent RFC process.
This part describes the RFC process
and archives all approvaed RFCs.

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
