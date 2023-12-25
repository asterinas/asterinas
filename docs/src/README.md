<!-- 
# Table of Content

1. Introduction
2. Design
   1. Privilege Separation
      1. Case Study 1: Syscall Workflow
      2. Case Study 2: Drivers for Virtio Devices on PCI
   2. Everything as a Capability
      1. Type-Level Programming in Rust
      2. CapComp: Zero-Cost Capabilities and Component
   3. (More content...)
-->

# Introduction

This document describes Asterinas, a secure, fast, and modern OS written in Rust.

As the project is a work in progress, this document is by no means complete.
Despite the incompleteness, this evolving document serves several important purposes:

1. To improve the quality of thinking by [putting ideas into words](http://www.paulgraham.com/words.html).
2. To convey the vision of this project to partners and stakeholders.
3. To serve as a blueprint for implementation.

## Opportunities

> The crazy people who are crazy enough to think they can change the world,
> are the ones who do. --Steve Jobs

We believe now is the perfect time to start a new Rust OS project. We argue that
if we are doing things right, the project can have a promising prospect to
success and a real shot of challenging the dominance of Linux in the long run.
Our confidence stems from the three technological, industrial, geo-political
trends.

First, [Rust](https://www.rust-lang.org/) is the future of system programming,
including OS development. Due to its advantages of safety, efficiency, and
productivity, Rust has been increasingly embraced by system developers,
including OS developers. [Linux is close to adopting Rust as an official
programming
language.](https://www.zdnet.com/article/linus-torvalds-is-cautiously-optimistic-about-bringing-rust-into-the-linux-kernels-next-release/)
Outside the Linux community, Rust enthusiasts are building from scratch new Rust
OSes, e.g., [Kerla](https://github.com/nuta/kerla),
[Occlum](https://github.com/occlum/occlum),
[Redox](https://github.com/redox-os/redox),
[rCore](https://github.com/rcore-os/rCore),
[RedLeaf](https://github.com/mars-research/redleaf),
[Theseus](https://github.com/theseus-os/Theseus),
and [zCore](https://github.com/rcore-os/zCore). Despite their varying degrees of
success, none of them are general-purpose, industrial-strength OSes that are or
will ever be competitive with Linux. Eventually, a winner will emerge out of this
market of Rust OSes, and Asterinas is our bet for this competition.

Second, Rust OSes are a perfect fit for
[Trusted Execution Environments (TEEs)](https://en.wikipedia.org/wiki/Trusted_execution_environment).
TEEs is an emerging hardware-based security technology that is expected to
become mainstream. All major CPU vendors have launched or announced their
implementations of VM-based TEEs, including
[ARM CCA](https://www.arm.com/architecture/security-features/arm-confidential-compute-architecture),
[AMD SEV](https://developer.amd.com/sev/),
[Intel TDX](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-trust-domain-extensions.html)
and [IBM PEF](https://research.ibm.com/publications/confidential-computing-for-openpower).
Typical applications that demand protection of TEEs also desire a TEE OS that is
more secure and trustworthy than Linux, the latter of which is plagued by the
inevitable security vulnerabilities resulting from the unsafe nature of C
language and the sheer complexity of the codebase. A new Rust OS built from
scratch is less likely to contain memory safety bugs and can enjoy a
significantly smaller Trusted Computing Base (TCB).

Third, the Chinese tech sector has a strong incentive to 
invest in local alternatives of critical software like OSes.
Based in China,
we have been observing greater aspiration of Chinese companies
as well as greater support from the Chinese government
to [achieve independency in key technologies like chips and software](https://www.nytimes.com/2021/03/10/business/china-us-tech-rivalry.html).
One success story of Chinese software independence is 
relational databases: 
[Oracle and IBM are losing ground as Chinese vendors catch up with their US counterparts](https://www.theregister.com/2022/07/06/international_database_vendors_are_losing/).
Can such success stories be repeated in the field of OSes? I think so.
There are some China's home-grown OSes like [openKylin](https://www.openkylin.top/index.php?lang=en), but all of them are based on Linux and lack a self-developed
OS _kernel_. The long-term goal of Asterinas is to fill this key missing core of the home-grown OSes.

## Architecture Overview

Here is an overview of the architecture of Asterinas.

![architecture overview](images/arch_overview.png)

## Features

**1. Security by design.** Security is our top priority in the design of Asterinas. As such, we adopt the widely acknowledged security best practice of [least privilege principle](https://en.wikipedia.org/wiki/Principle_of_least_privilege) and enforce it in a fashion that leverages the full strengths of Rust. To do so, we partition Asterinas into two halves: a _privileged_ OS core and _unprivileged_ OS components. All OS components are written entirely in _safe_ Rust and only the privileged OS core
is allowed to have _unsafe_ Rust code. Furthermore, we propose the idea of _everything-is-a-capability_, which elevates the status of [capabilities](https://en.wikipedia.org/wiki/Capability-based_security) to the level of a ubiquitous security primitive used throughout the OS. We make novel use of Rust's advanced features (e.g., [type-level programming](https://willcrichton.net/notes/type-level-programming/)) to make capabilities more accessible and efficient. The net result is improved security and uncompromised performance.

**2. Trustworthy OS-level virtualization.** OS-level virtualization mechanisms (like Linux's cgroups and namespaces) enable containers, a more lightweight and arguably more popular alternative to virtual machines (VMs). But there is one problem with containers: they are not as secure as VMs (see [StackExchange](https://security.stackexchange.com/questions/169642/what-makes-docker-more-secure-than-vms-or-bare-metal), [LWN](https://lwn.net/Articles/796700/), and [AWS](https://docs.aws.amazon.com/AmazonECS/latest/bestpracticesguide/security-tasks-containers.html)). There is a real risk that malicious containers may exploit privilege escalation bugs in the OS kernel to attack the host. [A study](https://dl.acm.org/doi/10.1145/3274694.3274720) found that 11 out of 88 kernel exploits are effective in breaking the container sandbox. The seemingly inherent insecurity of OS kernels leads to a new breed of container implementations (e.g., [Kata](https://katacontainers.io/) and [gVisor](https://gvisor.dev/)) that are based on VMs, instead of kernels, for isolation and sandboxing. We argue that this unfortunate retreat from OS-level virtualization to VM-based one is unwarranted---if the OS kernels are secure enough. And this is exactly what we plan to achieve with Asterinas. We aim to provide a trustworthy OS-level virtualization mechanism on Asterinas.

**3. Fast user-mode development.** Traditional OS kernels like Linux are hard to develop, test, and debug. Kernel development involves countless rounds of programming, failing, and rebooting on bare-metal or virtual machines. This way of life is unproductive and painful. Such a pain point is also recognized and partially addressed by [research work](https://www.usenix.org/conference/fast21/presentation/miller), but we think we can do more. In this spirit, we design the OS core to provide high-level APIs that are largely independent of the underlying hardware and implement it with two targets: one target is as part of a regular OS in kernel space and the other is as a library OS in user space. This way, all the OS components of Asterinas, which are stacked above the OS core, can be developed, tested, and debugged in user space, which is more friendly to developers than kernel space.

**4. High-fidelity Linux ABI.** An OS without usable applications is useless. So we believe it is important for Asterinas to fit in an established and thriving ecosystem of software, such as the one around Linux. This is why we conclude that Asterinas should aim at implementing high-fidelity Linux ABI, including the system calls, the proc file system, etc.

**5. TEEs as top-tier targets.** (Todo)

**6. Reservation-based OOM prevention.** (Todo)
