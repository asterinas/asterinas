# Example: Writing a Kernel in About 100 Lines of Safe Rust

To give you a sense of
how Asterinas OSTD enables writing kernels in safe Rust,
we will show a new kernel in about 100 lines of safe Rust.

Our new kernel will be able to run the following Hello World program.

```s
{{#include ../../../osdk/tests/examples_in_book/write_a_kernel_in_100_lines_templates/hello.S}}
```

The assembly program above can be compiled with the following command.

```bash
gcc -static -nostdlib hello.S -o hello
```

The user program above requires our kernel to support three main features:
1. Loading a program as a process image in user space;
3. Handling the write system call;
4. Handling the exit system call.

A sample implementation of the kernel in safe Rust is given below.
Comments are added
to highlight how the APIs of Asterinas OSTD enable safe kernel development.

```rust
{{#include ../../../osdk/tests/examples_in_book/write_a_kernel_in_100_lines_templates/lib.rs}}
```
