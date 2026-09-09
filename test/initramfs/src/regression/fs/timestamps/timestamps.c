// SPDX-License-Identifier: MPL-2.0

/*
 * Regression tests for signed VFS Unix timestamps.
 *
 * Pre-epoch `utimensat` / `stat` round-trips used to fail because inode
 * times were stored as `Duration`. See:
 * https://github.com/asterinas/asterinas/issues/3746
 */

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/statfs.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

#define EXFAT_MAGIC 0xaa55

#define PRE_EPOCH_ATIME (-1)
#define PRE_EPOCH_MTIME (-2051222400LL) /* 1905-01-01 */
#define I32_MIN ((int64_t)INT32_MIN)
#define I32_MAX ((int64_t)INT32_MAX)

static unsigned long fd_magic(int fd)
{
	struct statfs st;

	if (fstatfs(fd, &st) < 0) {
		return 0;
	}
	return (unsigned long)st.f_type;
}

static int supports_pre_epoch(unsigned long magic)
{
	return magic != EXFAT_MAGIC;
}

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret == 0 || errno == EEXIST);
	errno = 0;
}

/*
 * `TEST_*` macros update counters local to `FN_TEST`, so helpers that
 * assert must be macros expanded at the call site.
 */
#define TEST_PRE_EPOCH_ON_FD(fd)                                          \
	do {                                                              \
		int _fl = fcntl((fd), F_GETFL);                           \
		if (_fl >= 0) {                                           \
			fcntl((fd), F_SETFL, _fl | O_NOATIME);            \
			errno = 0;                                        \
		}                                                         \
		unsigned long _magic = fd_magic(fd);                      \
		struct timespec _times[2] = {                             \
			{ .tv_sec = PRE_EPOCH_ATIME, .tv_nsec = 0 },      \
			{ .tv_sec = PRE_EPOCH_MTIME, .tv_nsec = 0 },      \
		};                                                        \
		TEST_SUCC(utimensat((fd), "", _times, AT_EMPTY_PATH));    \
		struct stat _st;                                          \
		TEST_SUCC(fstat((fd), &_st));                             \
		if (supports_pre_epoch(_magic)) {                         \
			TEST_RES(_st.st_atim.tv_sec == PRE_EPOCH_ATIME && \
					 _st.st_mtim.tv_sec ==            \
						 PRE_EPOCH_MTIME,         \
				 _ret);                                   \
		} else {                                                  \
			TEST_RES(_st.st_atim.tv_sec >= 0 &&               \
					 _st.st_mtim.tv_sec >= 0,         \
				 _ret);                                   \
		}                                                         \
	} while (0)

#define TEST_PRE_EPOCH_ON_PATH(path)                                     \
	do {                                                             \
		int _fd = TEST_SUCC(                                     \
			open((path), O_RDWR | O_CREAT | O_TRUNC, 0644)); \
		TEST_PRE_EPOCH_ON_FD(_fd);                               \
		TEST_SUCC(close(_fd));                                   \
		TEST_SUCC(unlink((path)));                               \
	} while (0)

#define TEST_PRE_EPOCH_ON_DIR(path)                                        \
	do {                                                               \
		ensure_dir((path));                                        \
		int _fd = TEST_SUCC(open((path), O_RDONLY | O_DIRECTORY)); \
		TEST_PRE_EPOCH_ON_FD(_fd);                                 \
		TEST_SUCC(close(_fd));                                     \
		TEST_SUCC(rmdir((path)));                                  \
	} while (0)

#define TEST_NSEC_ON_PATH(path)                                          \
	do {                                                             \
		int _fd = TEST_SUCC(                                     \
			open((path), O_RDWR | O_CREAT | O_TRUNC, 0644)); \
		struct timespec _times[2] = {                            \
			{ .tv_sec = 10, .tv_nsec = 123 },                \
			{ .tv_sec = 11, .tv_nsec = 456 },                \
		};                                                       \
		TEST_SUCC(utimensat(_fd, "", _times, AT_EMPTY_PATH));    \
		struct stat _st;                                         \
		TEST_RES(fstat(_fd, &_st),                               \
			 _st.st_atim.tv_sec == 10 &&                     \
				 _st.st_atim.tv_nsec == 123 &&           \
				 _st.st_mtim.tv_sec == 11 &&             \
				 _st.st_mtim.tv_nsec == 456);            \
		TEST_SUCC(close(_fd));                                   \
		TEST_SUCC(unlink((path)));                               \
	} while (0)

#define TEST_I32_RANGE_ON_PATH(path)                                     \
	do {                                                             \
		int _fd = TEST_SUCC(                                     \
			open((path), O_RDWR | O_CREAT | O_TRUNC, 0644)); \
		struct timespec _times[2] = {                            \
			{ .tv_sec = (long)I32_MIN, .tv_nsec = 0 },       \
			{ .tv_sec = (long)I32_MAX, .tv_nsec = 0 },       \
		};                                                       \
		TEST_SUCC(utimensat(_fd, "", _times, AT_EMPTY_PATH));    \
		struct stat _st;                                         \
		TEST_RES(fstat(_fd, &_st),                               \
			 _st.st_atim.tv_sec == (long)I32_MIN &&          \
				 _st.st_mtim.tv_sec == (long)I32_MAX);   \
		TEST_SUCC(close(_fd));                                   \
		TEST_SUCC(unlink((path)));                               \
	} while (0)

FN_TEST(ramfs_pre_epoch_nsec_and_i32_range)
{
	const char *mnt = "/tmp/timestamps_ramfs";
	const char *file = "/tmp/timestamps_ramfs/file";
	const char *dir = "/tmp/timestamps_ramfs/dir";

	ensure_dir(mnt);
	TEST_SUCC(mount("none", mnt, "ramfs", 0, NULL));
	TEST_PRE_EPOCH_ON_PATH(file);
	TEST_NSEC_ON_PATH(file);
	TEST_I32_RANGE_ON_PATH(file);
	TEST_PRE_EPOCH_ON_DIR(dir);

	{
		const char *wide = "/tmp/timestamps_ramfs/beyond_i32";
		int fd =
			TEST_SUCC(open(wide, O_RDWR | O_CREAT | O_TRUNC, 0644));
		struct timespec times[2] = {
			{ .tv_sec = (long)(I32_MIN - 1), .tv_nsec = 0 },
			{ .tv_sec = (long)(I32_MAX + 1), .tv_nsec = 0 },
		};
		TEST_SUCC(utimensat(fd, "", times, AT_EMPTY_PATH));
		struct stat st;
		TEST_RES(fstat(fd, &st),
			 st.st_atim.tv_sec == (long)(I32_MIN - 1) &&
				 st.st_mtim.tv_sec == (long)(I32_MAX + 1));
		TEST_SUCC(close(fd));
		TEST_SUCC(unlink(wide));
	}
	TEST_SUCC(umount(mnt));
	TEST_SUCC(rmdir(mnt));
}
END_TEST()

FN_TEST(tmp_pre_epoch)
{
	TEST_PRE_EPOCH_ON_PATH("/tmp/timestamps_tmpfile");
	TEST_NSEC_ON_PATH("/tmp/timestamps_tmpfile_nsec");
	TEST_PRE_EPOCH_ON_DIR("/tmp/timestamps_tmpdir");
}
END_TEST()

FN_TEST(ext2_pre_epoch_persist_and_i32_range)
{
	SKIP_TEST_IF(access("/ext2", W_OK) != 0);

	TEST_PRE_EPOCH_ON_PATH("/ext2/timestamps_pre_epoch");
	TEST_I32_RANGE_ON_PATH("/ext2/timestamps_i32_range");

	const char *wide = "/ext2/timestamps_beyond_i32";
	struct timespec wide_times[2] = {
		{ .tv_sec = (long)(I32_MIN - 1), .tv_nsec = 123 },
		{ .tv_sec = (long)(I32_MAX + 1), .tv_nsec = 456 },
	};
	int wide_fd = TEST_SUCC(open(wide, O_RDWR | O_CREAT | O_TRUNC, 0644));
	TEST_SUCC(utimensat(wide_fd, "", wide_times, AT_EMPTY_PATH));
	struct stat wide_st;
	TEST_RES(fstat(wide_fd, &wide_st),
		 wide_st.st_atim.tv_sec == (long)I32_MIN &&
			 wide_st.st_atim.tv_nsec == 0 &&
			 wide_st.st_mtim.tv_sec == (long)I32_MAX &&
			 wide_st.st_mtim.tv_nsec == 0);
	TEST_SUCC(close(wide_fd));
	TEST_SUCC(unlink(wide));

	const char *path = "/ext2/timestamps_persist";
	struct timespec times[2] = {
		{ .tv_sec = PRE_EPOCH_ATIME, .tv_nsec = 0 },
		{ .tv_sec = PRE_EPOCH_MTIME, .tv_nsec = 0 },
	};
	int fd = TEST_SUCC(open(path, O_RDWR | O_CREAT | O_TRUNC, 0644));
	TEST_SUCC(utimensat(fd, "", times, AT_EMPTY_PATH));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(path, O_RDONLY));
	struct stat st;
	TEST_RES(fstat(fd, &st), st.st_atim.tv_sec == PRE_EPOCH_ATIME &&
					 st.st_mtim.tv_sec == PRE_EPOCH_MTIME);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(exfat_accepts_pre_epoch_without_einval)
{
	SKIP_TEST_IF(access("/exfat", W_OK) != 0);
	TEST_PRE_EPOCH_ON_PATH("/exfat/timestamps_pre_epoch");
}
END_TEST()

FN_TEST(overlayfs_pre_epoch)
{
	const char *base = "/tmp/timestamps_overlay";
	char lower[64], upper[64], work[64], merged[64], file[80], opts[256];

	snprintf(lower, sizeof(lower), "%s/lower", base);
	snprintf(upper, sizeof(upper), "%s/upper", base);
	snprintf(work, sizeof(work), "%s/work", base);
	snprintf(merged, sizeof(merged), "%s/merged", base);
	snprintf(file, sizeof(file), "%s/file", merged);

	ensure_dir(base);
	ensure_dir(lower);
	ensure_dir(upper);
	ensure_dir(work);
	ensure_dir(merged);

	snprintf(opts, sizeof(opts), "lowerdir=%s,upperdir=%s,workdir=%s",
		 lower, upper, work);
	TEST_SUCC(mount("overlay", merged, "overlay", 0, opts));
	TEST_PRE_EPOCH_ON_PATH(file);
	TEST_NSEC_ON_PATH(file);
	TEST_SUCC(umount(merged));
	TEST_SUCC(rmdir(merged));
	TEST_SUCC(rmdir(work));
	TEST_SUCC(rmdir(upper));
	TEST_SUCC(rmdir(lower));
	TEST_SUCC(rmdir(base));
}
END_TEST()

FN_TEST(pipe_pre_epoch)
{
	int fds[2];
	TEST_SUCC(pipe(fds));
	TEST_PRE_EPOCH_ON_FD(fds[0]);
	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
}
END_TEST()

FN_TEST(memfd_pre_epoch)
{
	int fd = TEST_SUCC(memfd_create("timestamps_memfd", 0));
	TEST_PRE_EPOCH_ON_FD(fd);
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(procfs_pre_epoch)
{
	int fd = TEST_SUCC(open("/proc/self", O_RDONLY | O_DIRECTORY));
	TEST_PRE_EPOCH_ON_FD(fd);
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(nsfs_pre_epoch)
{
	int fd = TEST_SUCC(open("/proc/self/ns/mnt", O_RDONLY));
	TEST_PRE_EPOCH_ON_FD(fd);
	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(devpts_pre_epoch)
{
	SKIP_TEST_IF(access("/dev/pts", R_OK) != 0);
	int fd = TEST_SUCC(open("/dev/pts", O_RDONLY | O_DIRECTORY));
	TEST_PRE_EPOCH_ON_FD(fd);
	TEST_SUCC(close(fd));
}
END_TEST()
