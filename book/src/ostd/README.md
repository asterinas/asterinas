# Asterinas OSTD

> Confucious remarked,
> "I could follow whatever my heart desired
> without transgressing the law."
>
> 子曰：
> "从心所欲，不逾矩。"

With the Asterinas OSTD (Operating System Standard Library), 
you don't have to learn the dark art of unsafe Rust programming
and risk shooting yourself in the foot.
You will be doing whatever your heart desires,
and be confident that your kernel will never crash
or be hacked due to undefined behaviors,
even if today marks your Day 1 as a Rust programmer.

## APIs

Asterinas OSTD stands
as a powerful and solid foundation for safe kernel development,
providing high-level safe Rust APIs that are

1. Essential for OS development, and
2. Dependent on the use of unsafe Rust.

Most of these APIs fall into the following categories:

* Memory management (e.g., allocating and accessing physical memory pages)
* Task management (e.g., context switching between kernel tasks)
* User space (e.g., manipulating and entering the user space)
* Interrupt handling (e.g., registering interrupt handlers)
* Timer management (e.g., registering timer handlers)
* Driver support (e.g., performing DMA and MMIO)
* Boot support (e.g., retrieving information from the bootloader)
* Synchronization (e.g., locking and sleeping)

To explore how these APIs come into play,
see [the example of a 100-line kernel in safe Rust](a-100-line-kernel.md).

The OSTD APIs have been extensively documented.
You can access the comprehensive API documentation by visiting the [docs.rs](https://docs.rs/ostd/latest/ostd).

## Four Requirements Satisfied

In designing and implementing OSTD,
we have risen to meet the challenge of
fulfilling [the aforementioned four criteria as demanded by the framekernel architecture](../kernel/the-framekernel-architecture.md).

Expressiveness is evident through Asterinas Kernel itself,
where all system calls,
file systems,
network protocols,
and device drivers (e.g., Virtio drivers)
have been implemented in safe Rust
by leveraging OSTD.

Adopting a minimalist philosophy,
OSTD has a small codebase.
At its core lies the `ostd` crate,
currently encompassing about 10K lines of code - a figure 
that is even smaller than those of many microkernels.
As OSTD evolves,
its codebase will expand,
albeit at a relatively slow rate
in comparison to the OS services layered atop it.

The OSTD's efficiency is measurable
through the performance metrics of its APIs
and the system calls of Asterinas Kernel.
No intrinsic limitations have been identified within Rust
or the framekernel architecture
that could hinder kernel performance.

Soundness, unlike the other three requirements,
is not as easily quantified or proved.
While formal verification stands as the gold standard,
it requires considerable resources and time
and is not an immediate priority.
As a more pragmatic approach,
we will explain why the high-level design is sound
in the soundness analysis and rely on the many 
eyes of the community to catch any potential flaws 
in the implementation.
