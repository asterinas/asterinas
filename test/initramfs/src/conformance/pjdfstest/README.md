# pjdfstest Conformance Suite

This directory contains the Asterinas integration for [pjdfstest](https://github.com/pjd/pjdfstest), a POSIX filesystem and syscall test suite. Asterinas packages pjdfstest into the initramfs, boots Asterinas in QEMU, and runs the selected cases in the guest against a filesystem of your choosing.

## How the Suite Works

pjdfstest is not a single test binary. It uses [TAP (Test Anything Protocol)](https://testanything.org/) as its case format. The suite has three parts:

- **The cases**: 238 shell scripts under the suite's `tests/` directory, named `<category>/<nn>.t` (for example `chmod/00.t`, `rename/22.t`).
- **The helper**: a small C program, `pjdfstest`, that each case shells out to. Every assertion invokes it with a syscall subcommand and compares the result against an expectation.
- **The driver**: Perl's [`prove`](https://perldoc.perl.org/prove), which runs the cases and reads their TAP output.

The cases are TAP: each prints lines such as `ok 1` or `not ok 7 - tried 'mkdir 2', expected EEXIST, got 0`. `prove` collects them into a `Test Summary Report`.

All test paths inside the cases are relative, so **the filesystem under test is whatever filesystem the working directory is on**. That is why the work directory matters more here than in the other conformance suites, and why the runner changes into it before starting `prove`.

## Directory Structure

```
pjdfstest/
|-- run_pjdfstest_test.sh   # Guest-side runner around `prove`
|-- conf                    # Environment for tests/misc.sh, and for the runner
|-- runlists/
|   |-- ramfs.list          # Cases selected on ramfs; suppressed ones commented out
|   `-- ext2.list           # Cases selected on ext2; suppressed ones commented out
`-- Makefile                # Copies the packaged suite into the initramfs
```

The upstream cases and the helper binary come from the Nix package. Only the hand-written files above live in this repository.

The case list is kept **per filesystem**, because the two implementations differ in ways the suite detects: a case that hangs or fails on one may pass on the other. The runner picks `runlists/<fs>.list` at runtime, with no configuration needed.

`conf` is the one place that decides which filesystem that is. It maps `PJDFSTEST_FS_TYPE` to the filesystem's work directory and names the result in `fs`; the runner sources the same file and uses that name to find the list, so the runner and the suite cannot disagree about which filesystem is under test.

## Running Tests

Run these commands from the project root inside the Asterinas development container.

```bash
# Run the whole suite against ext2 (the default)
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=pjdfstest

# Run the whole suite against the in-memory ramfs
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=pjdfstest \
    PJDFSTEST_FS_TYPE=ramfs

# Run a single case against ext2
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=pjdfstest \
    PJDFSTEST_FS_TYPE=ext2 CONFORMANCE_TEST_SELECTOR="chmod/00.t"

# Run several cases
make run_kernel AUTO_TEST=conformance CONFORMANCE_TEST_SUITE=pjdfstest \
    CONFORMANCE_TEST_SELECTOR="chmod/00.t,chmod/01.t,unlink/00.t"
```

## Choosing the Filesystem Under Test

`PJDFSTEST_FS_TYPE` selects the filesystem the suite runs on. Each value names the directory the suite runs in, and the case list to use there:

| `PJDFSTEST_FS_TYPE` | Work directory | Notes |
| --- | --- | --- |
| `ext2` (default) | `/ext2` | An ext2 image built by `test/initramfs/Makefile` |
| `ramfs` | `/ramfs` | An ordinary directory on the initramfs root, which is Asterinas' in-memory ramfs; discarded at poweroff |

## Select Test Cases to Run

`CONFORMANCE_TEST_SELECTOR` is a comma-separated list of cases to run which replaces the case list entirely. Intended for working on one or two cases at a time rather than for regular runs.

## Reading the Results

`prove -v` streams every assertion to the console as it happens, and the same output is written to `<$WORK_DIR>/pjdfstest.log`. Failed assertions are summarized separately in `<$WORK_DIR>/failed.txt`, and the runner prints them along with the `Test Summary Report` at the end.
Assertions marked `# TODO` are expected failures declared by the suite itself; they are excluded from `failed.txt` and do not affect the exit code.

If the run reports no test summary at all, the runner prints the head and tail of the log. That means no case started — a packaging problem rather than a test failure — and the excerpt is usually enough to identify it without booting again.

## Configuration

### `runlists/<fs>.list`

The cases a run selects, one `<category>/<nn>.t` per line. Blank lines and lines starting with `#` are ignored. The file for each filesystem lists every case the suite has, so the default run is the whole suite.

A case that does not pass on that filesystem is commented out in place, with the reason on the same line, rather than removed:

```text
chmod/00.t
# unlink/11.t: sticky bit not enforced on unlink
chmod/01.t
unlink/00.t
```

Keeping the name rather than deleting the line means the file still accounts for every case the suite ships, and each suppressed case stays visible.

### `conf`

Loaded by the suite's own `tests/misc.sh`, which every case sources. It sets `os`, `fs`, and `GREP`.

It also checks that the work directory it selects is actually on the filesystem it was asked for, and stops the run if it is not.

`os` and `fs` only affect the suite's own gating: a case may declare itself unsupported on a given filesystem, or mark an assertion as expected to fail, by comparing against them. `fs` is set by `conf` from `PJDFSTEST_FS_TYPE` — `rename/24.t` is the one case in the suite that reads it, to skip its Btrfs-specific expectations.

## Adding Tests

The case bodies come from the Nix package, so adding a case means selecting one that already exists upstream rather than writing one:

1. Add the case name, such as `chmod/00.t`, to `runlists/<fs>.list` for the filesystem you are working on.
2. Run it before submitting the change. A case that has never been run is not known to pass.
3. If it fails for a reason unrelated to the change under test, comment it out in place with a note saying why.

To find the full set of available cases, list the suite's `tests/` directory inside the guest, or read the Nix source pinned in `test/initramfs/nix/conformance/pjdfstest.nix`.

## Timeouts

Every case runs under a per-case wall-clock limit so that one wedged case cannot stall the whole run — without it, a case that hangs the kernel leaves the guest spinning until the CI job is killed and none of the remaining cases ever report.

The limit defaults to 300 seconds and can be overridden with `PJDFSTEST_TEST_TIMEOUT`. When a case exceeds it, `timeout` kills it and `prove` reports it as a failure, so a hang shows up as a *failed* case rather than a lost run. A hanging case still costs the full limit, so a run containing one takes correspondingly longer; raise the limit only for a case that is genuinely slow rather than stuck.
