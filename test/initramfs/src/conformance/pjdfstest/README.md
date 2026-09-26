# pjdfstest Conformance Suite

This directory contains the Asterinas integration for [pjdfstest](https://github.com/pjd/pjdfstest), a POSIX filesystem and syscall test suite. Asterinas packages pjdfstest into the initramfs, boots Asterinas in QEMU, and runs the selected cases in the guest against a filesystem of your choosing.

## How the Suite Works

pjdfstest is not a single test binary. It has three parts:

- **The cases**: 238 shell scripts under the suite's `tests/` directory, named `<category>/<nn>.t` (for example `chmod/00.t`, `rename/22.t`).
- **The helper**: a small C program, `pjdfstest`, that each case shells out to. Every assertion invokes it with a syscall subcommand and compares the result against an expectation.
- **The driver**: Perl's [`prove`](https://perldoc.perl.org/prove), which runs the cases and reads their TAP output.

The cases are TAP: each prints lines such as `ok 1` or `not ok 7 - tried 'mkdir 2', expected EEXIST, got 0`. `prove` collects them into a `Test Summary Report`.

All test paths inside the cases are relative, so **the filesystem under test is whatever filesystem the working directory is on**. That is why the work directory matters more here than in the other conformance suites, and why the runner changes into it before starting `prove`.

## Directory Structure

```
pjdfstest/
|-- run_pjdfstest_test.sh   # Guest-side runner around `prove`
|-- conf                    # Environment for the suite's tests/misc.sh
|-- runlist                 # Cases selected for a run; defaults to all 238
|-- blocklist               # Cases excluded from the run
`-- Makefile                # Copies the packaged suite into the initramfs
```

The upstream cases and the helper binary come from the Nix package. Only the hand-written files above live in this repository.

## Running Tests

Run these commands from the project root inside the Asterinas development container.

```bash
# Run the whole suite on the default work directory (/tmp)
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=pjdfstest

# Run a single case against the ext2 data disk
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=pjdfstest \
    CONFORMANCE_TEST_SELECTOR="chmod/00.t" \
    CONFORMANCE_TEST_WORKDIR=/ext2

# Run several cases
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=pjdfstest \
    CONFORMANCE_TEST_SELECTOR="chmod/00.t,chmod/01.t,unlink/00.t"
```

## Choosing the Filesystem Under Test

`CONFORMANCE_TEST_WORKDIR` selects the directory the suite runs in, and therefore the filesystem being tested. The initramfs mounts two data disks for exactly this purpose:

| `CONFORMANCE_TEST_WORKDIR` | Filesystem | Notes |
| --- | --- | --- |
| `/tmp` (default) | ramfs | In-memory; discarded at poweroff |
| `/ext2` | ext2 | A 2 GiB image built by `test/initramfs/Makefile` |
| `/exfat` | exfat | A 512 MiB image, mounted for the exfat tests |

The suite does more than read and write files: it depends on real filesystem semantics such as permissions, timestamps, hard links, and link counts. Running it on `/tmp` exercises the in-memory filesystem rather than ext2, so **use `CONFORMANCE_TEST_WORKDIR=/ext2` when you mean to test the ext2 implementation.**

A subdirectory of the work directory is used for the actual run (`<$CONFORMANCE_TEST_WORKDIR>/pjdfstest`), and it is removed and recreated on every run. Leaving scratch state behind would make later runs depend on earlier ones; several cases assert that a path does *not* exist, or that a directory is empty, and would fail spuriously otherwise.

## Select Test Cases to Run

`CONFORMANCE_TEST_SELECTOR` is a comma-separated list of cases to run instead of the `runlist`. Intended for working on one or two cases at a time rather than for regular runs.

Two behaviors worth knowing:

- **A selector replaces the `runlist` entirely.** It does not add to it.
- **A selector also bypasses the `blocklist`.** So a case you have blocked can still be rerun by naming it explicitly — otherwise a blocked case could never be investigated.

Every entry is validated against the suite before use, and a bad one stops the run with an error instead of being handed to `prove` as a path.

## Configuration

### `runlist`

The cases a run selects, one `<category>/<nn>.t` per line. Blank lines and lines starting with `#` are ignored. It ships listing every case the suite has, so the default run is the whole suite; narrow it down by removing lines:

```text
chmod/00.t
chmod/01.t
unlink/00.t
```

Because it is a checked-in file rather than a command-line variable, it is the natural place to record a curated set — a short list for regular runs, or a group of related cases you are working through.

### `blocklist`

Cases excluded from the run, in the same format. It is applied on top of whatever `runlist` selects, so a case listed in both does not run. Use it for cases that hang, panic the kernel, or fail for a reason that is known and being tracked.

### `conf`

Loaded by the suite's own `tests/misc.sh`, which every case sources. It sets `os`, `fs`, and `GREP`.

`os` and `fs` only affect the suite's own gating: a case may declare itself unsupported on a given filesystem, or mark an assertion as expected to fail, by comparing against them. `fs` is determined at runtime from `/proc/mounts` by matching `CONFORMANCE_TEST_WORKDIR` against the mount points.

## Reading the Results

The runner exits with:

| Code | Meaning |
| --- | --- |
| `0` | Every selected case passed |
| `1` | The suite ran, and at least one case failed |
| `2` | The run could not start — a missing file, a bad selector entry, or not running as root |

`prove -v` streams every assertion to the console as it happens, and the same output is written to `<$WORK_DIR>/pjdfstest.log`. Failed assertions are summarized separately in `<$WORK_DIR>/failed.txt`, and the runner prints them along with the `Test Summary Report` at the end.

Assertions marked `# TODO` are expected failures declared by the suite itself; they are excluded from `failed.txt` and do not affect the exit code.

If the run reports no test summary at all, the runner prints the head and tail of the log. That means no case started — a packaging problem rather than a test failure — and the excerpt is usually enough to identify it without booting again.

## Adding Tests

The case bodies come from the Nix package, so adding a case means selecting one that already exists upstream rather than writing one:

1. Add the case name, such as `chmod/00.t`, to `runlist`.
2. Run it before submitting the change. A case that has never been run is not known to pass.
3. If it fails for a reason unrelated to the change under test, add it to `blocklist` with a comment saying why.

To find the full set of available cases, list the suite's `tests/` directory inside the guest, or read the Nix source pinned in `test/initramfs/nix/conformance/pjdfstest.nix`.

## Limitations

- **The suite must run as root.** Many cases assert privileged behavior, and the runner refuses to start as a non-root user rather than reporting a wall of spurious failures.
