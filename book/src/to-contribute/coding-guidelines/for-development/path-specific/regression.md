# `test/initramfs/src/regression/`

Guidelines that apply only to code under `test/initramfs/src/regression`.

### Use the test framework for regression tests (`use-test-framework`) {#use-test-framework}

Write C regression tests under `test/` using the framework in
`test/initramfs/src/regression/common/test.h`.
Include this header and define each test case with `FN_TEST()` and `END_TEST()`.
The framework runs the tests, records assertion results,
and makes the program exit with a failure status when an assertion fails.
Use `FN_SETUP()` and `END_SETUP()` for shared setup and cleanup.
For test structure, see the
[maintainability guidelines for regression tests](../../for-maintainability/path-specific/regression.md).

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
For calls that return zero on success and a positive error code on failure,
such as `pthread_create()` and `pthread_join()`, use `CHECK_WITH(expr, _ret == 0)`.
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

Apply the same precision to setup and synchronization.
In setup code, use `CHECK_WITH` when a step requires a fixed number of bytes.
EOF or a short read fails that requirement even if the call does not set `errno`.

See also:
PR [#3109](https://github.com/asterinas/asterinas/pull/3109#discussion_r3056271232)
and [#3193](https://github.com/asterinas/asterinas/pull/3193#discussion_r3216193178);
[`unshare.c`](https://github.com/asterinas/asterinas/blob/318f374a770677ce971d6c7ee16f3f83083566eb/test/initramfs/src/regression/security/namespace/unshare.c).

### Keep operations and result checks in one assertion (`combine-test-assertions`) {#combine-test-assertions}

Keep the operation and its result checks in one assertion.
For example, when reading a synchronization byte,
check the returned byte count and the byte stored in the buffer together:

```c
// Good — keeps the read and both result checks together
TEST_RES(read(sync_fd, &byte, 1), _ret == 1 && byte == 1);

// Bad — checks the same results but separates the value check from the read
TEST_RES(read(sync_fd, &byte, 1), _ret == 1);
TEST_RES(0, byte == 1);
```

### Verify compatibility assertions on Linux (`verify-linux-behavior`) {#verify-linux-behavior}

After writing a compatibility assertion, run the test on Linux
to verify the expected behavior.
When Linux and Asterinas produce the same observable result,
use the same assertion on both systems.

If the results differ, check whether the assertion or test setup is wrong
before treating the difference as an Asterinas compatibility issue.
For a confirmed compatibility issue, consider fixing Asterinas,
especially when the fix is straightforward.

For a complex issue that cannot be resolved as part of the change,
use `#ifdef __asterinas__` in the test file to express different expectations.
Keep the conditional limited to the behavior that differs
and add a `FIXME` comment following the
[guideline below](#explain-test-differences).
Whenever possible, assert the current Asterinas behavior in one branch
and the Linux behavior in the other.
This makes later changes to Asterinas fail the old assertion,
prompting an update to the test or removal of the conditional and its `FIXME`
once the behaviors match.

See also:
the Linux and Asterinas cases in
[`proc_fd_open_fifo_after_setid.c`](https://github.com/asterinas/asterinas/blob/5f41ff02d681e047f9420bb18c162cfe76116606/test/initramfs/src/regression/fs/procfs/proc_fd_open_fifo_after_setid.c).

### Explain platform differences in FIXME comments (`explain-test-differences`) {#explain-test-differences}

Place a `FIXME` comment next to assertions that differ between Linux and Asterinas.
Explain what each system does,
why the test needs different expectations,
and what must change before the conditional can be removed.

For example, a test that sets its effective UID and GID to 65535
and clears dumpable can check `/proc/self/fd` ownership on both systems:

```c
struct stat st;

// Good — explains both outcomes and when to remove the conditional
/*
 * FIXME: Linux makes /proc/self/fd root-owned when dumpable is cleared.
 * Asterinas does not support dumpable and uses the effective UID/GID instead.
 * Remove the conditional assertions once Asterinas honors dumpable.
 */
#ifdef __asterinas__
TEST_RES(stat("/proc/self/fd", &st), st.st_uid == 65535 && st.st_gid == 65535);
#else
TEST_RES(stat("/proc/self/fd", &st), st.st_uid == 0 && st.st_gid == 0);
#endif

// Bad — leaves the reader to reconstruct the difference
// FIXME: Procfs ownership differs between Linux and Asterinas.
```

See also:
Linux man-pages [`proc_pid(5)`](https://man7.org/linux/man-pages/man5/proc_pid.5.html);
the platform-difference comment in
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

If the expected result is unknown, [verify it on Linux](#verify-linux-behavior).
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
see [shared setup and cleanup](../../for-maintainability/path-specific/regression.md#shared-test-fixtures).

Write cleanup as the final steps of the normal test sequence,
[assuming earlier assertions passed](../../for-maintainability/path-specific/regression.md#simple-test-flow).
There is no need for separate recovery branches to undo partial setup after a failure.

See also:
PR [#2926](https://github.com/asterinas/asterinas/pull/2926)
and [#2969](https://github.com/asterinas/asterinas/pull/2969).
