// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sched.h>
#include <unistd.h>
#include <stdlib.h>
#include <stdio.h>
#include <fcntl.h>
#include <errno.h>
#include <sys/syscall.h>

#include "../test.h"

FN_TEST(ns_file)
{
	const char *ns_path = "/proc/self/ns/user";
	int fd_ns = TEST_SUCC(open(ns_path, O_RDONLY));
	TEST_SUCC(close(fd_ns));

	char buf[128];
	ssize_t len;
	strcpy(buf, "/proc/self/ns/");
	ssize_t prefix_len = strlen(buf);
	len = TEST_RES(readlink(ns_path, buf + prefix_len,
				sizeof(buf) - prefix_len - 1),
		       strncmp("user:[", buf + prefix_len, 5) == 0);
	buf[prefix_len + len] = '\0';

	// The following test will fail on Asterinas because it currently permits
	// lookup of paths intended only for internal kernel use.
	// TEST_ERRNO(open(buf, O_RDONLY), ENOENT);
}
END_TEST()

FN_TEST(set_ns_empty_flags)
{
	const char *ns_path = "/proc/self/ns/user";
	int fd_ns = TEST_SUCC(open(ns_path, O_RDONLY));
	TEST_ERRNO(setns(fd_ns, 0), EINVAL);
	TEST_SUCC(close(fd_ns));

	pid_t pid = getpid();
	int pidfd = TEST_SUCC(syscall(SYS_pidfd_open, pid, 0));
	TEST_ERRNO(setns(pidfd, 0), EINVAL);
	TEST_SUCC(close(pidfd));
}
END_TEST()

FN_TEST(set_self_ns)
{
	// It is not permitted to use setns() to reenter the caller's
	// current user namespace. This is different from other namespaces.
	const char *ns_path = "/proc/self/ns/user";
	int fd_ns = TEST_SUCC(open(ns_path, O_RDONLY));
	TEST_ERRNO(setns(fd_ns, CLONE_NEWUSER), EINVAL);
	TEST_SUCC(close(fd_ns));

	pid_t pid = getpid();
	int pidfd = TEST_SUCC(syscall(SYS_pidfd_open, pid, 0));
	TEST_ERRNO(setns(pidfd, CLONE_NEWUSER), EINVAL);
	TEST_SUCC(close(pidfd));
}
END_TEST()
