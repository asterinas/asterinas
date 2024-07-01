# Asterinas OSTD

Asterinas OSTD is a Rust OS framework that facilitates the development of and innovation in OS kernels written in Rust.

## An overview

Asterinas OSTD provides a solid foundation for Rust developers to build their own OS kernels. While Asterinas OSTD origins from Asterinas, the first ever framekernel, Asterinas OSTD is well suited for building OS kernels of any architecture, be it a framekernel, a monolithic kernel, or a microkernel.

Asterinas OSTD offers the following key values.

1. **Lowering the entry bar for OS innovation.** Building an OS from scratch is not easy. Not to mention a novel one. Before adding any novel or interesting feature, an OS developer must first have something runnable, which must include basic functionalities for managing CPU, memory, and interrupts. Asterinas OSTD has laid this groundwork so that OS developers do not have to deal with the most low-level, error-prone, architecture-specific aspects of OS development themselves.

2. **Enhancing the memory safety of Rust OSes.** Asterinas OSTD encapsulates low-level, machine-oriented unsafe Rust code into high-level, machine-agnostic safe APIs. These APIs are carefully designed and implemented to be sound and minimal, ensuring the memory safety of any safe Rust callers. Our experience in building Asterinas has shown that Asterinas OSTD is powerful enough to allow a feature-rich, Linux-compatible kernel to be completely written in safe Rust, including its device drivers.

3. **Promoting code reuse across Rust OS projects.** Shipped as crates, Rust code can be reused across projects---except when they are OSes. A crate that implements a feature or driver for OS A can hardly be reused by OS B because the crate must be [`no_std`](https://docs.rust-embedded.org/book/intro/no-std.html#summary) and depend on the infrastructure APIs provided by OS A, which are obviously different from that provided by OS B. This incompatibility problem can be resolved by Asterinas OSTD as it can serve as a common ground across different Rust OS projects, as long as they are built upon Asterinas OSTD.

4. **Boost productivity with user-mode development.** Traditionally, developing a kernel feature involves countless rounds of coding, failing, and rebooting on bare-metal or virtual machines, which is a painfully slow process. Asterinas OSTD accelerates the process by allowing high-level OS features like file systems and network stacks to be quickly tested in user mode, making the experience of OS development as smooth as that of application development. To support user-mode development, Asterinas OSTD is implemented for the Linux platform, in addition to bare-mental or virtual machine environments.

## OSTD APIs

See [API docs](https://docs.rs/ostd/latest/ostd).
