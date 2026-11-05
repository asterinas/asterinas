# `test/`

These guidelines apply to C regression tests under `test/`
that use `test/initramfs/src/regression/common/test.h`.
For test structure, see the
[maintainability guidelines for `test/`](../../for-maintainability/path-specific/test.md).

### Choose assertions by execution context (`test-assertion-context`) {#test-assertion-context}

Prefer `TEST_*` for assertions in the main thread of an `FN_TEST` function.
These macros record assertion results and let the test continue after a failure.
Choose the macro according to what the operation should produce:

- `TEST_SUCC(expr)` checks that the operation succeeds.
- `TEST_ERRNO(expr, err)` checks that the operation sets the expected `errno`.
- `TEST_RES(expr, cond)` checks that the operation succeeds
  and its return value or output satisfies `cond`.
  Use `_ret` in the condition to refer to the return value.

This also applies to pthread calls wrapped by `test.h`,
such as `pthread_create()` and `pthread_join()`.
The wrappers copy returned error codes into `errno`,
so use `TEST_SUCC` when you expect the call to succeed.

Use `CHECK()` or `CHECK_WITH()` for code that must stop the process on failure:

- Setup and cleanup functions declared with `FN_SETUP`.
- Helpers that only prepare the environment, even when called from `FN_TEST`.
- Child processes and worker threads, including child branches inside `FN_TEST`.

Use `CHECK(expr)` for a call that must return a nonnegative value.
Use `CHECK_WITH(expr, cond)` when a step requires a more specific result,
such as reading exactly one synchronization byte.
Avoid these macros within the main test body and always use `TEST_*` instead.

In a forked test, the child's assertion counters are separate from the parent's.
Use `CHECK*` in the child and have the parent assert the child's exit status
so a child failure is reported as a test failure:

```c
FN_TEST(child_changes_directory)
{
    pid_t pid = TEST_SUCC(fork());
    if (pid == 0) {
        CHECK(chdir("/tmp"));
        exit(EXIT_SUCCESS);
    }

    int status;
    TEST_RES(waitpid(pid, &status, 0),
             _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()
```

Use the assertion macros to report results;
do not access framework counters such as `__tests_failed` directly.

See also:
PR [#4135](https://github.com/asterinas/asterinas/pull/4135#discussion_r4440242754)
and [#4090](https://github.com/asterinas/asterinas/pull/4090#discussion_r4439266461).

### Assert complete observable results (`complete-test-results`) {#complete-test-results}

Check every part of the result needed to establish the behavior under test.
Depending on the case, this can include the return value, output contents,
expected error, or relationship between objects.

Keep the operation and its result checks in one assertion when possible.
For example, when reading a synchronization byte,
check both the returned byte count and the byte stored in the buffer:

```c
// Good — checks both the byte count and the value
TEST_RES(read(sync_fd, &byte, 1), _ret == 1 && byte == 1);

// Bad — leaves the byte count unchecked and separates the value check
TEST_SUCC(read(sync_fd, &byte, 1));
TEST_RES(0, byte == 1);
```

Apply the same precision to setup and synchronization.
In setup code, use `CHECK_WITH` when a step requires a fixed number of bytes.
EOF or a short read fails that requirement even if the call does not set `errno`.

See also:
PR [#3109](https://github.com/asterinas/asterinas/pull/3109#discussion_r3056271232)
and [#3193](https://github.com/asterinas/asterinas/pull/3193#discussion_r3216193178);
[`unshare.c`](https://github.com/asterinas/asterinas/blob/318f374a770677ce971d6c7ee16f3f83083566eb/test/initramfs/src/regression/security/namespace/unshare.c).

### Establish Linux behavior before asserting compatibility (`verify-linux-behavior`) {#verify-linux-behavior}

Before writing a compatibility assertion, establish what Linux actually does:

1. Look for an existing test of the same behavior and check its expected result.
2. If the expected result is still unclear, run a minimal test on Linux to determine it.

Assert the result visible through the public API.
If Linux produces the same observable result for two cases,
use the same expectation for both,
even if the kernel handles them differently internally.

Add separate assertions for Linux and Asterinas only after confirming
that the same operation produces different results on the two systems.
In a [comment next to the assertions](../../for-maintainability/comments.md#design-decisions),
state what each system does and why the test needs different expectations.

See also:
the Linux and Asterinas cases in
[`proc_fd_open_fifo_after_setid.c`](https://github.com/asterinas/asterinas/blob/5f41ff02d681e047f9420bb18c162cfe76116606/test/initramfs/src/regression/fs/procfs/proc_fd_open_fifo_after_setid.c).

### Avoid unnecessary test skips (`avoid-test-skips`) {#avoid-test-skips}

Skip a test only when it cannot exercise the intended behavior
because a required feature or resource is unavailable.
Explain which requirement is missing.

Before adding a skip, check whether the test can still run:

- If an unsupported or ignored feature is only part of setup,
  use an equivalent supported mechanism when one exists.
- If an environment difference does not affect the behavior under test,
  run the test in that environment too.
  For example, a concurrency test that remains meaningful on one CPU
  should not check the CPU count just to skip that configuration.

If the expected result is unknown, [determine it](#verify-linux-behavior)
before writing the assertion.
Do not use an unconditional skip such as `SKIP_TEST_IF(1)` to leave that question unresolved.

See also:
PR [#3778](https://github.com/asterinas/asterinas/pull/3778#discussion_r4420916016).

### Clean up resources after use (`test-cleanup`) {#test-cleanup}

Release resources acquired by a test before it finishes:
close file descriptors, unlink temporary files,
join worker threads, and reap child processes with `waitpid`.
Leftover resources can interfere with later tests and cause intermittent failures.

```c
// Good — cleanup after use
int fd = TEST_SUCC(open("/tmp/test_file", O_CREAT | O_RDWR, 0644));

// ... test logic ...

TEST_SUCC(close(fd));
TEST_SUCC(unlink("/tmp/test_file"));
```

Clean up shared fixtures after the last test that uses them;
see [shared setup and cleanup](../../for-maintainability/path-specific/test.md#shared-test-fixtures).

Write cleanup as the final steps of the normal test sequence,
[assuming earlier assertions passed](../../for-maintainability/path-specific/test.md#simple-test-flow).
There is no need for separate recovery branches to undo partial setup after a failure.

See also:
PR [#2926](https://github.com/asterinas/asterinas/pull/2926)
and [#2969](https://github.com/asterinas/asterinas/pull/2969).
