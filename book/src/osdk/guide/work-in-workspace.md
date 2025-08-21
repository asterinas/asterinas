# Working in a Workspace

Typically, an operating system may consist of multiple crates,
and these crates may be organized in a workspace.
The OSDK also supports managing projects in a workspace.
Below is an example that demonstrates
how to create, build, run, and test projects in a workspace.

## Creating a new workspace

Create a new workspace by executing the following commands:

```bash
mkdir myworkspace && cd myworkspace
touch Cargo.toml
```

Then, add the following content to `Cargo.toml`:

```toml
{{#include ../../../../osdk/tests/examples_in_book/work_in_workspace_templates/Cargo.toml}}
```

## Creating a kernel project and a library project

The two projects can be created using the following commands:

```bash
cargo osdk new --kernel myos
cargo osdk new mylib
```

The generated directory structure will be as follows:

```text
myworkspace/
  ├── Cargo.toml
  ├── OSDK.toml
  ├── rust-toolchain.toml
  ├── myos/
  │   ├── Cargo.toml
  │   └── src/
  │       └── lib.rs
  └── mylib/
      ├── Cargo.toml
      └── src/
          └── lib.rs
```

At present, the OSDK mandates that there must be only one kernel project
within a workspace.

In addition to the two projects,
the OSDK will also generate `OSDK.toml` and `rust-toolchain.toml`
at the root of the workspace.

Next, add the following function to `mylib/src/lib.rs`.
This function will calculate the available memory
after booting:

```rust
{{#include ../../../../osdk/tests/examples_in_book/work_in_workspace_templates/mylib/src/lib.rs}}
```

Then, add a dependency on `mylib` to `myos/Cargo.toml`:

```toml
{{#include ../../../../osdk/tests/examples_in_book/work_in_workspace_templates/myos/Cargo.toml}}
```

In `myos/src/lib.rs`,
modify the file content as follows.
This main function will call the function from `mylib`:

```rust
{{#include ../../../../osdk/tests/examples_in_book/work_in_workspace_templates/myos/src/lib.rs}}
```

## Building and Running the kernel

Build and run the project using the following commands:

```bash
cargo osdk build
cargo osdk run
```

If everything goes well,
you will see the output from the guest kernel.

## Running unit test

You can run test cases from all crates
by using the following command in the workspace folder:

```bash
cargo osdk test
```

If you want to run test cases from a specific crate,
navigate to the crate's folder
and run `cargo osdk test`.
For example, if you want to test `mylib`,
use the following command:

```bash
cd mylib && cargo osdk test
```
