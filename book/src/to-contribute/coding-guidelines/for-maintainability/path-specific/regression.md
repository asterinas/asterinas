# `test/initramfs/src/regression/`

Guidelines that apply only to code under `test/initramfs/src/regression`.

### Keep test control flow simple (`simple-test-flow`) {#simple-test-flow}

Write each test as a straight sequence of operations and assertions,
with each step assuming that earlier assertions passed.
Assert cleanup operations in the same way.
Avoid branches that repeat an assertion's check
just to decide whether to continue:

```c
// Good — the operation and cleanup are asserted directly
int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
TEST_SUCC(ftruncate(fd, 0));
TEST_SUCC(close(fd));

// Bad — skips assertions after a failed prerequisite
int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
if (fd >= 0) {
    TEST_SUCC(ftruncate(fd, 0));
    TEST_SUCC(close(fd));
}
```

In setup helpers, use `CHECK()` to stop execution when setup fails.
In both setup helpers and test routines, avoid saving `errno`,
undoing partially completed operations, `goto cleanup`,
helpers that close a descriptor only if it is valid,
and variables that track which steps succeeded,
unless required by the behavior being tested.

Do not add retry loops or partial-I/O recovery helpers
unless retrying or recovering from partial I/O is the behavior under test.
Similarly, use a short sleep when it is enough to let a peer become ready;
use explicit synchronization when the test requires a specific ordering.

Keeping control flow simple does not remove the need to
[check complete results](../../for-development/path-specific/regression.md#complete-test-results)
and [release resources](../../for-development/path-specific/regression.md#test-cleanup).

See also:
PR [#4077](https://github.com/asterinas/asterinas/pull/4077#discussion_r4386190034),
[#4063](https://github.com/asterinas/asterinas/pull/4063#discussion_r4400420596),
and [#3109](https://github.com/asterinas/asterinas/pull/3109#discussion_r3061920087).

### Use fixed test identifiers (`fixed-test-identifiers`) {#fixed-test-identifiers}

Regression tests assume a single run in a clean environment.
Use readable, fixed paths, directory names, and other test identifiers
so readers can see which resources a test uses.
Do not construct paths from `TEST_TMPDIR`
or generate unique names with PIDs, timestamps, randomness, or `mkdtemp()`
just to avoid collisions.

```c
// Good — the fixed directory is immediately visible
CHECK(mkdir("/tmp/test-fixture", 0755));

// Bad — adds uniqueness that the test does not need
char path[64];
snprintf(path, sizeof(path), "/tmp/test-fixture-%d", getpid());
CHECK(mkdir(path, 0755));
```

Generate an identifier only when the behavior being tested needs a runtime value,
such as a `/proc/<pid>` path for a process created by the test.

If a failed run leaves files or directories behind,
remove them outside the test before running it again.
Do not add startup cleanup just to accommodate those leftovers.

See also:
PR [#4077](https://github.com/asterinas/asterinas/pull/4077#discussion_r4386501525).

### Share setup and cleanup across tests (`shared-test-fixtures`) {#shared-test-fixtures}

When several tests in a C file use the same environment,
prepare it once in `FN_SETUP(init)` using `CHECK()` calls.
This shared setup can create directories and mount points
or configure process settings.
Each `FN_TEST` can then focus on the behavior it tests.

Release shared resources and restore process settings in `FN_SETUP(cleanup)`.
The framework runs `FN_SETUP` and `FN_TEST` functions in definition order,
so place initialization before the tests and cleanup after the last test that uses it:

```c
#define TEST_DIR "/tmp/test-fixture"

FN_SETUP(init)
{
    CHECK(mkdir(TEST_DIR, 0755));
}
END_SETUP()

FN_TEST(directory_exists)
{
    struct stat st;
    TEST_RES(stat(TEST_DIR, &st), S_ISDIR(st.st_mode));
}
END_TEST()

// Other tests using TEST_DIR go here, before cleanup.

FN_SETUP(cleanup)
{
    CHECK(rmdir(TEST_DIR));
}
END_SETUP()
```

Keep setup inside an individual test when the setup operation is itself under test,
or when that test needs a separate environment.

See also:
PR [#3109](https://github.com/asterinas/asterinas/pull/3109#discussion_r3063532324).

### Use helpers to make test intent clear (`semantic-test-helpers`) {#semantic-test-helpers}

When repeated operations and result checks make a test hard to follow,
extract them into a helper.
This lets readers follow the test's actions and expected results
without working through the same buffer declarations and length calculations each time.

Choose a name that tells readers what the helper does.
For example, `TEST_SETSOCKOPT_INT` sets an integer socket option,
and `TEST_NL_GROUPS` checks a socket's Netlink group membership.
Pass the inputs and expected results as arguments
so readers can understand each case from the helper calls.

A helper that makes assertions can be a macro wrapping `TEST_*`.
The macro handles temporary buffers, length arguments, and repeated checks,
while recording assertions in the calling `FN_TEST`.
For example, the following test adds and removes group membership,
checking the expected group mask after each operation:

```c
// Good — the membership changes and expected group masks are easy to follow
TEST_SETSOCKOPT_INT(sock_fd, NETLINK_ADD_MEMBERSHIP, 1);
TEST_NL_GROUPS(sock_fd, 1);

TEST_SETSOCKOPT_INT(sock_fd, NETLINK_DROP_MEMBERSHIP, 1);
TEST_NL_GROUPS(sock_fd, 0);

// Bad — repeats buffer setup and result checks for each membership change
int group = 1;
TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK, NETLINK_ADD_MEMBERSHIP,
                     &group, sizeof(group)));
struct sockaddr_nl addr;
socklen_t addr_len = sizeof(addr);
TEST_RES(getsockname(sock_fd, (struct sockaddr *)&addr, &addr_len),
         addr_len == sizeof(addr) && addr.nl_groups == 1);

TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK, NETLINK_DROP_MEMBERSHIP,
                     &group, sizeof(group)));
addr_len = sizeof(addr);
TEST_RES(getsockname(sock_fd, (struct sockaddr *)&addr, &addr_len),
         addr_len == sizeof(addr) && addr.nl_groups == 0);
```

A test helper must preserve all assertion checks.
For example, `TEST_NL_GROUPS` checks both the returned address length
and the expected group mask.
Keep details in the test body when they define the case being tested:
a test of short socket-option buffers should show the buffer length explicitly.

Before writing a helper, check whether an existing one serves the same purpose.
Keep new helpers focused on operations that recur in the tests;
avoid adding options or abstractions that no test needs.

See also:
PR [#4027](https://github.com/asterinas/asterinas/pull/4027#discussion_r4421970440)
and its [socket-option helper review](https://github.com/asterinas/asterinas/pull/4027#discussion_r4409068743);
[`netlink_sockoption.c`](https://github.com/asterinas/asterinas/blob/39dd442cb45f24a0b6f16d7598b92b1c7aabb950/test/initramfs/src/regression/network/netlink_sockoption.c#L26-L73)
and [`netlink_sockoption.h`](https://github.com/asterinas/asterinas/blob/39dd442cb45f24a0b6f16d7598b92b1c7aabb950/test/initramfs/src/regression/network/netlink_sockoption.h#L12-L26).
