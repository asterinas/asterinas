// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <sys/syscall.h>
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

FN_SETUP(cleanup)
{
	CHECK(unlink(TEST_FILE));
}
END_SETUP()
