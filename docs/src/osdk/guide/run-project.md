# Testing or Running an OS Project

OSDK allows for convenient building, running,
and testing of an OS project.
The following example shows the typical workflow.

Suppose you have created a new kernel project named `myos`
and you are in the project directory:

```bash
cargo osdk new --kernel myos && cd myos
```

## Build the project

To build the project and its dependencies,
simply type:

```bash
cargo osdk build
```

## Run the project

To launch the kernel with QEMU,
use the following command:

```bash
cargo osdk run
```

OSDK will boot the kernel
and initialize OS resources like the console for output,
and then hand over control to the kernel entry point
to execute the kernel code.

**Note**: Only kernel projects (the projects
that defines the function marked with `#[aster_main]`)
can be run;
library projects cannot.


## Test the project

To run the kernel mode tests, use the following command:

```bash
cargo osdk test
```

OSDK will run all the kernel mode tests in the crate.

If you want to run a specific test with a given name,
for example, if the test is named `foo`,
use the following command:

```bash
cargo osdk test foo
```

## Options

Both `build`, `run`, and `test` commands accept options
to control their behavior, such as how to compile and
launch the kernel.
The following documentations provide details on
all the available options:

- [build options](../reference/commands/build.md)
- [run options](../reference/commands/run.md)
- [test options](../reference/commands/test.md)
