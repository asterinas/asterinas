// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <linux/falloc.h>
#include <signal.h>
#include <stdint.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

FN_SETUP(create_file)
{
	int fd = CHECK(
		open("/tmp/ramfs_fallocate", O_RDWR | O_CREAT | O_TRUNC, 0644));
	CHECK(close(fd));
}
END_SETUP()

FN_TEST(write_checks_offset_and_size_limits)
{
	int fd = TEST_SUCC(open("/tmp/ramfs_fallocate", O_RDWR));

	// Invalid signed offset ranges fail without extending the file.
	TEST_RES(lseek(fd, INT64_MAX, SEEK_SET), _ret == INT64_MAX);
	TEST_RES(write(fd, "", 0), _ret == 0);
	TEST_ERRNO(write(fd, "x", 1), EINVAL);
	TEST_ERRNO(pwrite(fd, "x", 1, INT64_MAX), EINVAL);
	TEST_RES(lseek(fd, 0, SEEK_CUR), _ret == INT64_MAX);

	struct stat st;
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 0);

	// The requested offset is checked before O_APPEND selects EOF.
	int append_fd =
		TEST_SUCC(open("/tmp/ramfs_fallocate", O_WRONLY | O_APPEND));
	TEST_RES(lseek(append_fd, INT64_MAX, SEEK_SET), _ret == INT64_MAX);
	TEST_ERRNO(write(append_fd, "x", 1), EINVAL);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 0);
	TEST_RES(lseek(append_fd, 0, SEEK_SET), _ret == 0);

	// A valid requested offset can still exceed the filesystem's size limit.
	struct rlimit old;
	TEST_SUCC(getrlimit(RLIMIT_FSIZE, &old));
	struct rlimit limit = { RLIM_INFINITY, old.rlim_max };
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &limit));
	TEST_SUCC(ftruncate(fd, INT64_MAX));
	TEST_ERRNO(write(append_fd, "x", 1), EFBIG);
	TEST_RES(lseek(append_fd, 0, SEEK_CUR), _ret == 0);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == INT64_MAX);
	TEST_SUCC(ftruncate(fd, 0));
	TEST_RES(write(append_fd, "x", 1), _ret == 1);
	TEST_RES(lseek(append_fd, 0, SEEK_CUR), _ret == 1);

	// Crossing the size limit writes the remaining byte.
	TEST_SUCC(ftruncate(fd, INT64_MAX - 1));
	TEST_RES(lseek(append_fd, 0, SEEK_SET), _ret == 0);
	TEST_RES(write(append_fd, "xy", 2), _ret == 1);
	TEST_RES(lseek(append_fd, 0, SEEK_CUR), _ret == INT64_MAX);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == INT64_MAX);
	char byte;
	TEST_RES(pread(fd, &byte, 1, INT64_MAX - 1), _ret == 1 && byte == 'x');

	// An empty append leaves the descriptor offset and file size unchanged.
	TEST_RES(lseek(append_fd, 7, SEEK_SET), _ret == 7);
	TEST_RES(write(append_fd, "", 0), _ret == 0);
	TEST_RES(lseek(append_fd, 0, SEEK_CUR), _ret == 7);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == INT64_MAX);
	TEST_SUCC(ftruncate(fd, 0));
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &old));
	TEST_SUCC(close(append_fd));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(allocate_checks_signed_range)
{
	int fd = TEST_SUCC(open("/tmp/ramfs_fallocate", O_RDWR | O_TRUNC));
	struct rlimit old;
	TEST_SUCC(getrlimit(RLIMIT_FSIZE, &old));
	struct rlimit limit = { RLIM_INFINITY, old.rlim_max };
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &limit));

	static const int modes[] = {
		0,
		FALLOC_FL_KEEP_SIZE,
		FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
	};
	for (size_t i = 0; i < sizeof(modes) / sizeof(modes[0]); i++) {
		TEST_ERRNO(fallocate(fd, modes[i], INT64_MAX, 1), EFBIG);
	}
	TEST_ERRNO(fallocate(-1, 0, INT64_MAX, 1), EBADF);
	struct stat st;
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 0);
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &old));
	TEST_SUCC(close(fd));
}
END_TEST()

// `fallocate` refuses a range that exceeds `RLIMIT_FSIZE`.
FN_TEST(allocate_respects_rlimit_fsize)
{
	// Ignore SIGXFSZ so Linux returns EFBIG to the test.
	struct sigaction action = { .sa_handler = SIG_IGN };
	struct sigaction old_action;
	TEST_SUCC(sigemptyset(&action.sa_mask));
	TEST_SUCC(sigaction(SIGXFSZ, &action, &old_action));

	struct rlimit old;
	TEST_SUCC(getrlimit(RLIMIT_FSIZE, &old));

	struct rlimit limit = { 4096, old.rlim_max };
	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &limit));

	int fd = TEST_SUCC(open("/tmp/ramfs_fallocate", O_RDWR));
	TEST_ERRNO(fallocate(fd, 0, 0, 8192), EFBIG);
	TEST_SUCC(fallocate(fd, 0, 0, 4096));
	TEST_SUCC(close(fd));

	TEST_SUCC(setrlimit(RLIMIT_FSIZE, &old));
	TEST_SUCC(sigaction(SIGXFSZ, &old_action, NULL));
}
END_TEST()

FN_SETUP(remove_file)
{
	CHECK(unlink("/tmp/ramfs_fallocate"));
}
END_SETUP()
