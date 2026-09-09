// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_FILE "/tmp/fcntl_owner_regression"

FN_SETUP(create)
{
	int fd = CHECK(open(TEST_FILE, O_CREAT | O_RDWR | O_TRUNC, 0666));
	CHECK(close(fd));
}
END_SETUP()

FN_TEST(dup_shares_owner)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
	int duplicated_fd = TEST_SUCC(dup(fd));
	int separate_fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
	pid_t pid = TEST_SUCC(getpid());

	TEST_SUCC(fcntl(fd, F_SETOWN, pid));
	TEST_RES(syscall(SYS_fcntl, duplicated_fd, F_GETOWN, 0), _ret == pid);
	TEST_RES(syscall(SYS_fcntl, separate_fd, F_GETOWN, 0), _ret == 0);

	TEST_SUCC(fcntl(duplicated_fd, F_SETOWN, 0));
	TEST_RES(syscall(SYS_fcntl, fd, F_GETOWN, 0), _ret == 0);

	TEST_SUCC(close(separate_fd));
	TEST_SUCC(close(duplicated_fd));
	TEST_SUCC(close(fd));
}
END_TEST()

// `F_SETOWN` takes a process group ID as a negative value, and `F_GETOWN`
// reports it back as a negative value. The raw syscall is used because the
// glibc wrapper mistakes small negative return values for error codes.
FN_TEST(setown_accepts_process_group)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
	pid_t pgid = TEST_SUCC(getpgrp());

	TEST_SUCC(fcntl(fd, F_SETOWN, -pgid));
	TEST_RES(syscall(SYS_fcntl, fd, F_GETOWN, 0), _ret == -pgid);

	// Setting a process owner afterwards replaces the process group owner.
	TEST_SUCC(fcntl(fd, F_SETOWN, getpid()));
	TEST_RES(syscall(SYS_fcntl, fd, F_GETOWN, 0), _ret == getpid());

	TEST_SUCC(close(fd));
}
END_TEST()

// A process group owner must not be confused with the process that happens to
// share its ID. The child below is the leader of its own group, so `-pgid` and
// `pgid` name different owners.
FN_TEST(process_and_group_owners_are_distinct)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));

	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		// Become the leader of a new process group and wait to be killed.
		setpgid(0, 0);
		pause();
		_exit(0);
	}

	// The parent also sets the child's process group, so the test does not
	// depend on which process is scheduled first.
	TEST_SUCC(setpgid(child, child));

	TEST_SUCC(fcntl(fd, F_SETOWN, child));
	TEST_RES(syscall(SYS_fcntl, fd, F_GETOWN, 0), _ret == child);

	TEST_SUCC(fcntl(fd, F_SETOWN, -child));
	TEST_RES(syscall(SYS_fcntl, fd, F_GETOWN, 0), _ret == -child);

	TEST_SUCC(kill(child, SIGKILL));
	TEST_SUCC(waitpid(child, NULL, 0));

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(setown_rejects_unknown_and_out_of_range_ids)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));

	// `INT_MAX` is far above the largest PID the kernel hands out, so it
	// names neither a live process nor a live process group.
	TEST_ERRNO(fcntl(fd, F_SETOWN, INT_MAX), ESRCH);
	TEST_ERRNO(fcntl(fd, F_SETOWN, -INT_MAX), ESRCH);

	// `INT_MIN` has no positive counterpart and must be rejected rather
	// than negated.
	TEST_ERRNO(fcntl(fd, F_SETOWN, INT_MIN), EINVAL);

	// A rejected `F_SETOWN` leaves the previous owner untouched.
	TEST_RES(syscall(SYS_fcntl, fd, F_GETOWN, 0), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(unlink(TEST_FILE));
}
END_SETUP()
