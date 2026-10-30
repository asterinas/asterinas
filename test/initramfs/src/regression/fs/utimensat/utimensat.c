// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#include <utime.h>

#include "../../common/test.h"

#define TEST_FILE "/tmp/utimensat_at_empty_path"

static int fd = -1;

FN_SETUP(prepare)
{
	fd = CHECK(open(TEST_FILE, O_CREAT | O_RDWR | O_TRUNC, 0644));
}
END_SETUP()

FN_TEST(both_omitted_ignores_path_and_flags)
{
	struct timespec times[2] = {
		{ .tv_sec = 1234567899, .tv_nsec = UTIME_OMIT },
		{ .tv_sec = 1234567898, .tv_nsec = UTIME_OMIT },
	};
	TEST_SUCC(utimensat(AT_FDCWD, TEST_FILE, times, 0xdeadbeef));
	TEST_SUCC(utimensat(AT_FDCWD, "/tmp/nonexistent_file", times, 0));
}
END_TEST()

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

#define NON_OWNER_UID 1000

static void become_non_owner(void)
{
	CHECK(setresgid(NON_OWNER_UID, NON_OWNER_UID, NON_OWNER_UID));
	CHECK(setresuid(NON_OWNER_UID, NON_OWNER_UID, NON_OWNER_UID));
}

static int child_exit_ok(pid_t pid)
{
	int status;

	return waitpid(pid, &status, 0) == pid && WIFEXITED(status) &&
	       WEXITSTATUS(status) == 0;
}

FN_TEST(non_owner_without_write_cannot_touch)
{
	TEST_SUCC(chown(TEST_FILE, 0, 0));
	TEST_SUCC(chmod(TEST_FILE, 0644));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		errno = 0;
		_exit(utime(TEST_FILE, NULL) == -1 && errno == EACCES ? 0 : 1);
	}
	TEST_RES(child_exit_ok(pid), _ret);
}
END_TEST()

FN_TEST(non_owner_with_write_can_touch)
{
	TEST_SUCC(chown(TEST_FILE, 0, 0));
	TEST_SUCC(chmod(TEST_FILE, 0666));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		errno = 0;
		_exit(utime(TEST_FILE, NULL) == 0 ? 0 : 1);
	}
	TEST_RES(child_exit_ok(pid), _ret);
}
END_TEST()

FN_TEST(owner_without_write_can_touch)
{
	TEST_SUCC(chown(TEST_FILE, NON_OWNER_UID, NON_OWNER_UID));
	TEST_SUCC(chmod(TEST_FILE, 0444));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		errno = 0;
		_exit(utime(TEST_FILE, NULL) == 0 ? 0 : 1);
	}
	TEST_RES(child_exit_ok(pid), _ret);
}
END_TEST()

FN_TEST(two_utime_now_values_are_a_touch_request)
{
	struct timespec times[2] = {
		{ .tv_sec = 0, .tv_nsec = UTIME_NOW },
		{ .tv_sec = 0, .tv_nsec = UTIME_NOW },
	};

	TEST_SUCC(chown(TEST_FILE, 0, 0));
	TEST_SUCC(chmod(TEST_FILE, 0644));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		errno = 0;
		_exit(utimensat(AT_FDCWD, TEST_FILE, times, 0) == -1 &&
			      errno == EACCES
			  ? 0
			  : 1);
	}
	TEST_RES(child_exit_ok(pid), _ret);
}
END_TEST()

FN_TEST(non_owner_cannot_set_explicit_times)
{
	struct timespec times[2] = {
		{ .tv_sec = 100, .tv_nsec = 0 },
		{ .tv_sec = 200, .tv_nsec = 0 },
	};

	TEST_SUCC(chown(TEST_FILE, 0, 0));
	TEST_SUCC(chmod(TEST_FILE, 0666));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		errno = 0;
		_exit(utimensat(AT_FDCWD, TEST_FILE, times, 0) == -1 &&
			      errno == EPERM
			  ? 0
			  : 1);
	}
	TEST_RES(child_exit_ok(pid), _ret);
}
END_TEST()

FN_TEST(cap_fowner_can_set_explicit_times)
{
	struct timespec times[2] = {
		{ .tv_sec = 100, .tv_nsec = 0 },
		{ .tv_sec = 200, .tv_nsec = 0 },
	};

	/* The regression test process is root but not this inode's owner. */
	TEST_SUCC(chown(TEST_FILE, NON_OWNER_UID, NON_OWNER_UID));
	TEST_SUCC(chmod(TEST_FILE, 0000));
	TEST_SUCC(utimensat(AT_FDCWD, TEST_FILE, times, 0));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(fd));
	CHECK(unlink(TEST_FILE));
}
END_SETUP()
