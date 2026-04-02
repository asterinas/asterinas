// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sched.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/syscall.h>

#include "../../common/test.h"

FN_TEST(set_ns_empty_flags)
{
	// FIXME: The following test will fail on Asterinas
	// because it currently does not support ns file.
#ifndef __asterinas__
	const char *ns_path = "/proc/self/ns/user";
	int fd_ns = TEST_SUCC(open(ns_path, O_RDONLY));
	TEST_ERRNO(setns(fd_ns, 0), EINVAL);
	TEST_SUCC(close(fd_ns));
#endif

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
	// FIXME: The following test will fail on Asterinas
	// because it currently does not support ns file.
#ifndef __asterinas__
	const char *ns_path = "/proc/self/ns/user";
	int fd_ns = TEST_SUCC(open(ns_path, O_RDONLY));
	TEST_ERRNO(setns(fd_ns, CLONE_NEWUSER), EINVAL);
	TEST_SUCC(close(fd_ns));
#endif

	pid_t pid = getpid();
	int pidfd = TEST_SUCC(syscall(SYS_pidfd_open, pid, 0));
	TEST_ERRNO(setns(pidfd, CLONE_NEWUSER), EINVAL);
	TEST_SUCC(close(pidfd));
}
END_TEST()
