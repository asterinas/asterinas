// SPDX-License-Identifier: MPL-2.0

// `struct f_owner_ex` and the `F_*OWN_EX` commands are GNU extensions.
#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_FILE "/tmp/fcntl_owner_regression"

// Reads `F_GETOWN` and undoes glibc's errno translation.
//
// glibc turns any raw syscall return in [-4095, -1] into -1 with `errno` set, so a
// process group owner -- which `F_GETOWN` reports negated -- comes back as an errno
// rather than as a value. That ambiguity is exactly why `F_GETOWN_EX` exists; here
// the original return is reconstructed so the negation itself can be asserted.
static long getown_raw(int fd)
{
	long ret;

	errno = 0;
	ret = syscall(SYS_fcntl, fd, F_GETOWN, 0);
	if (ret == -1 && errno != 0) {
		ret = -errno;
		errno = 0;
	}

	return ret;
}

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
	TEST_RES(getown_raw(duplicated_fd), _ret == pid);
	TEST_RES(getown_raw(separate_fd), _ret == 0);

	TEST_SUCC(fcntl(duplicated_fd, F_SETOWN, 0));
	TEST_RES(getown_raw(fd), _ret == 0);

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
	TEST_RES(getown_raw(fd), _ret == -pgid);

	// Setting a process owner afterwards replaces the process group owner.
	TEST_SUCC(fcntl(fd, F_SETOWN, getpid()));
	TEST_RES(getown_raw(fd), _ret == getpid());

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
	TEST_RES(getown_raw(fd), _ret == child);

	TEST_SUCC(fcntl(fd, F_SETOWN, -child));
	TEST_RES(getown_raw(fd), _ret == -child);

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
	TEST_RES(getown_raw(fd), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

// `F_SETOWN_EX` takes the ID as a positive value for every owner kind; the `type` field
// carries what `F_SETOWN` encodes in the sign.
FN_TEST(setown_ex_round_trips_each_owner_kind)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
	struct f_owner_ex owner;

	owner.type = F_OWNER_PID;
	owner.pid = getpid();
	TEST_SUCC(fcntl(fd, F_SETOWN_EX, &owner));

	memset(&owner, 0, sizeof(owner));
	TEST_SUCC(fcntl(fd, F_GETOWN_EX, &owner));
	TEST_RES(owner.type, _ret == F_OWNER_PID);
	TEST_RES(owner.pid, _ret == getpid());

	// A process group is reported positively here, unlike through `F_GETOWN`.
	owner.type = F_OWNER_PGRP;
	owner.pid = getpgrp();
	TEST_SUCC(fcntl(fd, F_SETOWN_EX, &owner));

	memset(&owner, 0, sizeof(owner));
	TEST_SUCC(fcntl(fd, F_GETOWN_EX, &owner));
	TEST_RES(owner.type, _ret == F_OWNER_PGRP);
	TEST_RES(owner.pid, _ret == getpgrp());

	// ... while `F_GETOWN` still negates it, which is exactly the ambiguity
	// `F_GETOWN_EX` exists to resolve.
	TEST_RES(getown_raw(fd), _ret == -getpgrp());

	owner.type = F_OWNER_TID;
	owner.pid = syscall(SYS_gettid);
	TEST_SUCC(fcntl(fd, F_SETOWN_EX, &owner));

	memset(&owner, 0, sizeof(owner));
	TEST_SUCC(fcntl(fd, F_GETOWN_EX, &owner));
	TEST_RES(owner.type, _ret == F_OWNER_TID);
	TEST_RES(owner.pid, _ret == syscall(SYS_gettid));

	TEST_SUCC(close(fd));
}
END_TEST()

// A file description that was never given an owner reports `F_OWNER_TID` with a zero ID,
// and clearing an owner keeps the recorded type while zeroing the ID.
FN_TEST(getown_ex_reports_the_recorded_type_without_an_owner)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
	struct f_owner_ex owner;

	memset(&owner, 0xff, sizeof(owner));
	TEST_SUCC(fcntl(fd, F_GETOWN_EX, &owner));
	TEST_RES(owner.type, _ret == F_OWNER_TID);
	TEST_RES(owner.pid, _ret == 0);

	// Clearing through `F_SETOWN` records the process kind, not the thread kind.
	TEST_SUCC(fcntl(fd, F_SETOWN, 0));
	memset(&owner, 0xff, sizeof(owner));
	TEST_SUCC(fcntl(fd, F_GETOWN_EX, &owner));
	TEST_RES(owner.type, _ret == F_OWNER_PID);
	TEST_RES(owner.pid, _ret == 0);

	// Clearing through `F_SETOWN_EX` keeps whichever kind was asked for.
	owner.type = F_OWNER_PGRP;
	owner.pid = 0;
	TEST_SUCC(fcntl(fd, F_SETOWN_EX, &owner));
	memset(&owner, 0xff, sizeof(owner));
	TEST_SUCC(fcntl(fd, F_GETOWN_EX, &owner));
	TEST_RES(owner.type, _ret == F_OWNER_PGRP);
	TEST_RES(owner.pid, _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(setown_ex_rejects_invalid_arguments)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
	struct f_owner_ex owner;

	owner.type = 12345;
	owner.pid = getpid();
	TEST_ERRNO(fcntl(fd, F_SETOWN_EX, &owner), EINVAL);

	// The sign is not a way to select a process group here, so a negative ID simply
	// names nothing rather than being rejected as malformed.
	owner.type = F_OWNER_PID;
	owner.pid = -getpid();
	TEST_ERRNO(fcntl(fd, F_SETOWN_EX, &owner), ESRCH);

	owner.type = F_OWNER_TID;
	owner.pid = INT_MAX;
	TEST_ERRNO(fcntl(fd, F_SETOWN_EX, &owner), ESRCH);

	owner.type = F_OWNER_PGRP;
	owner.pid = INT_MAX;
	TEST_ERRNO(fcntl(fd, F_SETOWN_EX, &owner), ESRCH);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(unlink(TEST_FILE));
}
END_SETUP()
