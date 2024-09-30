# Why OSDK

OSDK is designed to elevate the development experience
for Rust OS developers to the ease and convenience
typically associated with Rust application development.
Imagine crafting operating systems
with the same simplicity as applications!
This is important to Asterinas
as we believe that the project's success
is intricately tied to the productivity and happiness
of its developers.
So the OSDK is here to upgrade your dev experience.

To be honest, writing OS kernels is hard.
Even when you're using Rust,
which is a total game-changer for OS devs,
the challenge stands tall.
There is a bunch of reasons.

First, it is hard to write a new kernel from scratch.
Everything that has been taken for granted by
application developers are gone:
no stack, no heap, no threads, not even the standard I/O.
It's just you and the no_std world of Rust.
You have to implement these basic programming primitives
by getting your hands dirty with the most low-level,
error-prone, and nitty-gritty details of computer architecture.
It's a journey of learning, doing, and
a whole lot of finger-crossing
to make sure everything clicks into place.
This means a high entry bar for new OS creators.

Second, it is hard to
reuse OS-related libraries/crates across projects.
Think about it:
most applications share a common groundwork,
like libc, Rust's std library, or an SDK.
This isn't the case with kernels -
they lack this shared starting point,
forcing each one to craft its own set of tools
from the ground up.
Take device drivers, for example.
They often need DMA-capable buffers for chatting with hardware,
but since every kernel has its own DMA API flavor,
a driver for one kernel is pretty much a no-go for another.
This means that for each new kernel out there,
developers find themselves having to 'reinvent the wheel'
for many core components that are standard in other kernels.

Third, it is hard to do unit tests for OS functionalities.
Unit testing plays a crucial role in ensuring code quality,
but when you're dealing with a monolithic kernel like Linux,
it's like a spaghetti bowl of intertwined parts.
Trying to isolate one part for testing?
Forget about it.
You'd have to boot the whole kernel just to test a slice of it.
Loadable kernel modules are no exception:
you can't test them without plugging them into a live kernel.
This monolithic approach to unit testing is slow and unproductive
as it performs the job of unit tests
at a price of integration tests.
Regardless of the kernel architecture,
Rust's built-in unit testing facility
is not suited for kernel development,
leaving each kernel to hack together their own testing frameworks.

Last, it is hard to avoid writing unsafe Rust in a Rust kernel.
Rust brings safety...
well, at least for Rust applications,
where you can pretty much stay
in the wonderland of safe Rust all the way through.
But for a Rust kernel,
one cannot help but use unsafe Rust.
This is because, among other reasons,
low-level operations (e.g., managing page tables,
doing context switching, handling interrupts,
and interacting with devices) have to be expressed
with unsafe Rust features (like executing assembly code
or dereferencing raw pointers).
The misuse of unsafe Rust could lead to
various safety and security issues,
as reported by [RustSec Advisory Database](https://rustsec.org).
Despite having [a whole book](https://doc.rust-lang.org/nomicon/)
to document "the Dark Arts of Unsafe Rust",
unsafe Rust is still tricky to use correctly,
even among seasoned Rust developers.
