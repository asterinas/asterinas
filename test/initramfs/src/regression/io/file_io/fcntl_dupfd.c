// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <sys/resource.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(dupfd_respects_rlimit_nofile)
{
	struct rlimit limit;
	int fd = TEST_SUCC(open("/dev/null", O_RDONLY));

	if (TEST_SUCC(getrlimit(RLIMIT_NOFILE, &limit)) < 0) {
		TEST_SUCC(close(fd));
		return;
	}

	int duplicated_fd =
		TEST_RES(fcntl(fd, F_DUPFD, (int)(limit.rlim_cur - 1)),
			 _ret >= 0 && (unsigned long)_ret < limit.rlim_cur);
	if (duplicated_fd >= 0)
		TEST_SUCC(close(duplicated_fd));

	TEST_ERRNO(fcntl(fd, F_DUPFD, (int)limit.rlim_cur), EINVAL);
	TEST_ERRNO(fcntl(fd, F_DUPFD_CLOEXEC, (int)limit.rlim_cur), EINVAL);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(dupfd_returns_emfile_when_limit_is_reached)
{
	struct rlimit original_limit;
	int fd = TEST_SUCC(open("/dev/null", O_RDONLY));
	if (TEST_SUCC(getrlimit(RLIMIT_NOFILE, &original_limit)) < 0) {
		TEST_SUCC(close(fd));
		return;
	}

	struct rlimit limit = original_limit;
	if (limit.rlim_cur > 16) {
		limit.rlim_cur = 16;
		if (TEST_SUCC(setrlimit(RLIMIT_NOFILE, &limit)) < 0) {
			TEST_SUCC(close(fd));
			return;
		}
	}

	int *duplicated_fds = malloc(limit.rlim_cur * sizeof(*duplicated_fds));
	CHECK(duplicated_fds != NULL ? 0 : -1);
	size_t num_duplicated_fds = 0;
	while (num_duplicated_fds < limit.rlim_cur) {
		int duplicated_fd = fcntl(fd, F_DUPFD, 0);
		if (duplicated_fd < 0)
			break;
		duplicated_fds[num_duplicated_fds++] = duplicated_fd;
	}

	TEST_ERRNO(fcntl(fd, F_DUPFD, 0), EMFILE);

	for (size_t i = 0; i < num_duplicated_fds; i++)
		TEST_SUCC(close(duplicated_fds[i]));
	free(duplicated_fds);
	if (limit.rlim_cur != original_limit.rlim_cur)
		TEST_SUCC(setrlimit(RLIMIT_NOFILE, &original_limit));
	TEST_SUCC(close(fd));
}
END_TEST()
