// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <linux/stat.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_FILE "/tmp/utimensat_at_empty_path"

static int fd = -1;

FN_SETUP(prepare)
{
	fd = CHECK(open(TEST_FILE, O_CREAT | O_RDWR | O_TRUNC, 0644));
}
END_SETUP()

FN_TEST(at_empty_path_updates_times)
{
	struct timespec times[2] = {
		{ .tv_sec = 1234567890, .tv_nsec = 0 },
		{ .tv_sec = 1234567891, .tv_nsec = 0 },
	};
	TEST_SUCC(utimensat(fd, "", times, AT_EMPTY_PATH));

	struct stat st;
	TEST_RES(fstat(fd, &st), st.st_atim.tv_sec == 1234567890 &&
					 st.st_mtim.tv_sec == 1234567891);
}
END_TEST()

/* Regression for https://github.com/asterinas/asterinas/issues/3746 */
FN_TEST(negative_tv_sec_roundtrip)
{
	struct timespec times[2] = {
		{ .tv_sec = -1, .tv_nsec = 0 },
		{ .tv_sec = -2051222400, .tv_nsec = 0 }, /* 1905-01-01 */
	};
	TEST_SUCC(utimensat(fd, "", times, AT_EMPTY_PATH));

	struct stat st;
	TEST_RES(fstat(fd, &st),
		 st.st_atim.tv_sec == -1 && st.st_mtim.tv_sec == -2051222400);
}
END_TEST()

FN_TEST(empty_path_without_flag_returns_enoent)
{
	struct timespec times[2] = {
		{ .tv_sec = 1, .tv_nsec = 0 },
		{ .tv_sec = 1, .tv_nsec = 0 },
	};
	TEST_ERRNO(utimensat(fd, "", times, 0), ENOENT);
}
END_TEST()

FN_TEST(legacy_null_pathname)
{
	struct timespec times[2] = {
		{ .tv_sec = 1000000000, .tv_nsec = 0 },
		{ .tv_sec = 1000000001, .tv_nsec = 0 },
	};
	TEST_SUCC(syscall(SYS_utimensat, fd, NULL, times, 0));

	struct stat st;
	TEST_RES(fstat(fd, &st), st.st_atim.tv_sec == 1000000000 &&
					 st.st_mtim.tv_sec == 1000000001);
}
END_TEST()

FN_TEST(legacy_null_pathname_with_nonzero_flag_returns_einval)
{
	struct timespec times[2] = {
		{ .tv_sec = 1, .tv_nsec = 0 },
		{ .tv_sec = 1, .tv_nsec = 0 },
	};
	TEST_ERRNO(syscall(SYS_utimensat, fd, NULL, times, AT_SYMLINK_NOFOLLOW),
		   EINVAL);
}
END_TEST()

FN_TEST(legacy_null_pathname_with_o_path_returns_ebadf)
{
	int opath_fd = TEST_SUCC(open(TEST_FILE, O_PATH));
	struct timespec times[2] = {
		{ .tv_sec = 10, .tv_nsec = 0 },
		{ .tv_sec = 20, .tv_nsec = 0 },
	};

	TEST_ERRNO(syscall(SYS_utimensat, opath_fd, NULL, times, 0), EBADF);
	TEST_SUCC(syscall(SYS_utimensat, opath_fd, "", times, AT_EMPTY_PATH));

	TEST_SUCC(close(opath_fd));
}
END_TEST()

FN_TEST(at_empty_path_on_o_path_symlink_updates_symlink)
{
	const char *symlink_path = "/tmp/utimensat_symlink";
	TEST_SUCC(symlink(TEST_FILE, symlink_path));
	int sym_fd = TEST_SUCC(open(symlink_path, O_PATH | O_NOFOLLOW));

	struct timespec times[2] = {
		{ .tv_sec = 1500000000, .tv_nsec = 0 },
		{ .tv_sec = 1500000001, .tv_nsec = 0 },
	};
	TEST_SUCC(utimensat(sym_fd, "", times,
			    AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW));

	struct stat sym_st;
	TEST_RES(fstatat(AT_FDCWD, symlink_path, &sym_st, AT_SYMLINK_NOFOLLOW),
		 sym_st.st_atim.tv_sec == 1500000000 &&
			 sym_st.st_mtim.tv_sec == 1500000001);

	TEST_SUCC(close(sym_fd));
	TEST_SUCC(unlink(symlink_path));
}
END_TEST()

FN_TEST(epoch_and_nanoseconds)
{
	struct timespec times[2] = {
		{ .tv_sec = 0, .tv_nsec = 1 },
		{ .tv_sec = 0, .tv_nsec = 999999999 },
	};
	TEST_SUCC(utimensat(fd, "", times, AT_EMPTY_PATH));

	struct stat st;
	TEST_RES(fstat(fd, &st), st.st_atim.tv_sec == 0 &&
					 st.st_atim.tv_nsec == 1 &&
					 st.st_mtim.tv_sec == 0 &&
					 st.st_mtim.tv_nsec == 999999999);
}
END_TEST()

FN_TEST(utime_omit_preserves_atime)
{
	struct timespec seed[2] = {
		{ .tv_sec = 100, .tv_nsec = 0 },
		{ .tv_sec = 200, .tv_nsec = 0 },
	};
	TEST_SUCC(utimensat(fd, "", seed, AT_EMPTY_PATH));

	struct timespec times[2] = {
		{ .tv_sec = 0, .tv_nsec = UTIME_OMIT },
		{ .tv_sec = 300, .tv_nsec = 0 },
	};
	TEST_SUCC(utimensat(fd, "", times, AT_EMPTY_PATH));

	struct stat st;
	TEST_RES(fstat(fd, &st),
		 st.st_atim.tv_sec == 100 && st.st_mtim.tv_sec == 300);
}
END_TEST()

FN_TEST(both_omit_is_noop)
{
	struct timespec seed[2] = {
		{ .tv_sec = 111, .tv_nsec = 0 },
		{ .tv_sec = 222, .tv_nsec = 0 },
	};
	TEST_SUCC(utimensat(fd, "", seed, AT_EMPTY_PATH));

	struct timespec times[2] = {
		{ .tv_sec = 0, .tv_nsec = UTIME_OMIT },
		{ .tv_sec = 0, .tv_nsec = UTIME_OMIT },
	};
	TEST_SUCC(utimensat(fd, "", times, AT_EMPTY_PATH));

	struct stat st;
	TEST_RES(fstat(fd, &st),
		 st.st_atim.tv_sec == 111 && st.st_mtim.tv_sec == 222);
}
END_TEST()

FN_TEST(utime_now_sets_current_time)
{
	struct timespec times[2] = {
		{ .tv_sec = 0, .tv_nsec = UTIME_NOW },
		{ .tv_sec = 0, .tv_nsec = UTIME_NOW },
	};
	time_t before = TEST_SUCC(time(NULL));
	TEST_SUCC(utimensat(fd, "", times, AT_EMPTY_PATH));
	time_t after = TEST_SUCC(time(NULL));

	struct stat st;
	TEST_SUCC(fstat(fd, &st));
	TEST_RES(st.st_mtim.tv_sec >= before - 1, _ret);
	TEST_RES(st.st_mtim.tv_sec <= after + 1, _ret);
}
END_TEST()

FN_TEST(invalid_nsec_returns_einval)
{
	struct timespec times[2] = {
		{ .tv_sec = 1, .tv_nsec = 1000000000 },
		{ .tv_sec = 1, .tv_nsec = 0 },
	};
	TEST_ERRNO(utimensat(fd, "", times, AT_EMPTY_PATH), EINVAL);
}
END_TEST()

FN_TEST(futimesat_accepts_negative_timeval)
{
	struct timeval times[2] = {
		{ .tv_sec = -1, .tv_usec = 0 },
		{ .tv_sec = 1, .tv_usec = 0 },
	};
	TEST_SUCC(syscall(SYS_futimesat, AT_FDCWD, TEST_FILE, times));

	struct stat st;
	TEST_RES(fstat(fd, &st),
		 st.st_atim.tv_sec == -1 && st.st_mtim.tv_sec == 1);
}
END_TEST()

FN_TEST(statx_reports_negative_atime)
{
	struct timespec times[2] = {
		{ .tv_sec = -1, .tv_nsec = 0 },
		{ .tv_sec = -2, .tv_nsec = 0 },
	};
	TEST_SUCC(utimensat(fd, "", times, AT_EMPTY_PATH));

	struct statx stx;
	TEST_SUCC(
		statx(fd, "", AT_EMPTY_PATH, STATX_ATIME | STATX_MTIME, &stx));
	TEST_RES(stx.stx_atime.tv_sec == -1 && stx.stx_mtime.tv_sec == -2,
		 _ret);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(fd));
	CHECK(unlink(TEST_FILE));
}
END_SETUP()
