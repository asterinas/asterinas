// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define ARRAY_SIZE(array) (sizeof(array) / sizeof((array)[0]))
#define CHILD_COUNT 16
#define EXTRA_FD_COUNT 3
#define MAX_DIRENTS 128
#define MID_DIRENT_BUF_SIZE 256
#define NORMAL_DIRENT_BUF_SIZE 512
#define SMALL_DIRENT_BUF_SIZE 128

struct linux_dirent64 {
	uint64_t d_ino;
	int64_t d_off;
	unsigned short d_reclen;
	unsigned char d_type;
	char d_name[];
};

struct proc_dirent {
	char name[256];
	unsigned char type;
	int64_t next_off;
};

static int extra_fds[EXTRA_FD_COUNT];

static size_t append_dirents(struct proc_dirent *entries, size_t count,
			     void *buf, ssize_t bytes)
{
	for (size_t pos = 0; pos < (size_t)bytes;) {
		struct linux_dirent64 *dirent =
			(struct linux_dirent64 *)((char *)buf + pos);

		snprintf(entries[count].name, sizeof(entries[count].name), "%s",
			 dirent->d_name);
		entries[count].type = dirent->d_type;
		entries[count].next_off = dirent->d_off;
		count++;
		pos += dirent->d_reclen;
	}

	return count;
}

static int is_dot_dirent(const char *name)
{
	return strcmp(name, ".") == 0 || strcmp(name, "..") == 0;
}

static unsigned char dirent_type_from_mode(mode_t mode)
{
	if (S_ISREG(mode)) {
		return DT_REG;
	}
	if (S_ISDIR(mode)) {
		return DT_DIR;
	}
	if (S_ISLNK(mode)) {
		return DT_LNK;
	}
	if (S_ISCHR(mode)) {
		return DT_CHR;
	}
	if (S_ISBLK(mode)) {
		return DT_BLK;
	}
	if (S_ISFIFO(mode)) {
		return DT_FIFO;
	}
	if (S_ISSOCK(mode)) {
		return DT_SOCK;
	}
	return DT_UNKNOWN;
}

static int same_visible_dirent(const struct proc_dirent *lhs,
			       const struct proc_dirent *rhs)
{
	return strcmp(lhs->name, rhs->name) == 0 && lhs->type == rhs->type;
}

FN_SETUP(open_extra_proc_fds)
{
	for (size_t i = 0; i < ARRAY_SIZE(extra_fds); i++) {
		extra_fds[i] = CHECK(dup(STDIN_FILENO));
	}
}
END_SETUP()

FN_TEST(proc_self_dirent_types_match_stat)
{
	/*
	 * Scans `/proc/self` with `getdents64` and verifies that every visible
	 * directory entry reports the same file type as `fstatat` sees without
	 * following symlinks.
	 */
	struct proc_dirent entries[MAX_DIRENTS];
	char buf[NORMAL_DIRENT_BUF_SIZE];
	size_t count = 0;
	int fd = TEST_SUCC(open("/proc/self", O_RDONLY | O_DIRECTORY));

	for (;;) {
		ssize_t bytes =
			TEST_RES(syscall(SYS_getdents64, fd, buf, sizeof(buf)),
				 _ret >= 0);
		if (bytes <= 0) {
			break;
		}

		count = append_dirents(entries, count, buf, bytes);
	}

	for (size_t i = 0; i < count; i++) {
		struct stat stat_buf;

		if (is_dot_dirent(entries[i].name)) {
			continue;
		}

		TEST_RES(fstatat(fd, entries[i].name, &stat_buf,
				 AT_SYMLINK_NOFOLLOW),
			 entries[i].type ==
				 dirent_type_from_mode(stat_buf.st_mode));
	}

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(proc_self_task_dirent_types_match_stat)
{
	/*
	 * Scans `/proc/self/task` with `getdents64` and checks that task
	 * directory entries expose a `d_type` consistent with their `fstatat`
	 * file type.
	 */
	struct proc_dirent entries[MAX_DIRENTS];
	char buf[NORMAL_DIRENT_BUF_SIZE];
	size_t count = 0;
	int fd = TEST_SUCC(open("/proc/self/task", O_RDONLY | O_DIRECTORY));

	for (;;) {
		ssize_t bytes =
			TEST_RES(syscall(SYS_getdents64, fd, buf, sizeof(buf)),
				 _ret >= 0);
		if (bytes <= 0) {
			break;
		}

		count = append_dirents(entries, count, buf, bytes);
	}

	for (size_t i = 0; i < count; i++) {
		struct stat stat_buf;

		if (is_dot_dirent(entries[i].name)) {
			continue;
		}

		TEST_RES(fstatat(fd, entries[i].name, &stat_buf,
				 AT_SYMLINK_NOFOLLOW),
			 entries[i].type ==
				 dirent_type_from_mode(stat_buf.st_mode));
	}

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(proc_self_fd_dirent_types_match_stat)
{
	/*
	 * Scans `/proc/self/fd` while several descriptors are open and verifies
	 * that each file-descriptor entry's `d_type` matches `fstatat`.
	 */
	struct proc_dirent entries[MAX_DIRENTS];
	char buf[NORMAL_DIRENT_BUF_SIZE];
	size_t count = 0;
	int fd = TEST_SUCC(open("/proc/self/fd", O_RDONLY | O_DIRECTORY));

	for (;;) {
		ssize_t bytes =
			TEST_RES(syscall(SYS_getdents64, fd, buf, sizeof(buf)),
				 _ret >= 0);
		if (bytes <= 0) {
			break;
		}

		count = append_dirents(entries, count, buf, bytes);
	}

	for (size_t i = 0; i < count; i++) {
		struct stat stat_buf;

		if (is_dot_dirent(entries[i].name)) {
			continue;
		}

		TEST_RES(fstatat(fd, entries[i].name, &stat_buf,
				 AT_SYMLINK_NOFOLLOW),
			 entries[i].type ==
				 dirent_type_from_mode(stat_buf.st_mode));
	}

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(proc_self_fd_d_off_resumes_from_next_entry)
{
	/*
	 * Reads a prefix of `/proc/self/fd`, seeks to the last entry's `d_off`,
	 * and verifies that the resumed scan equals the suffix of a full scan.
	 */
	struct proc_dirent full_scan[MAX_DIRENTS];
	struct proc_dirent prefix[MAX_DIRENTS];
	struct proc_dirent resumed[MAX_DIRENTS];
	char normal_buf[NORMAL_DIRENT_BUF_SIZE];
	char small_buf[SMALL_DIRENT_BUF_SIZE];
	size_t full_count = 0;
	size_t prefix_count = 0;
	size_t resumed_count = 0;
	int fd = TEST_SUCC(open("/proc/self/fd", O_RDONLY | O_DIRECTORY));

	ssize_t prefix_bytes = TEST_RES(syscall(SYS_getdents64, fd, small_buf,
						sizeof(small_buf)),
					_ret > 0);

	prefix_count =
		append_dirents(prefix, prefix_count, small_buf, prefix_bytes);
	TEST_RES(prefix_count, _ret > 0);

	int64_t next_off = prefix[prefix_count - 1].next_off;
	TEST_RES(lseek(fd, 0, SEEK_SET), _ret == 0);

	for (;;) {
		ssize_t bytes = TEST_RES(syscall(SYS_getdents64, fd, normal_buf,
						 sizeof(normal_buf)),
					 _ret >= 0);
		if (bytes <= 0) {
			break;
		}

		full_count = append_dirents(full_scan, full_count, normal_buf,
					    bytes);
	}

	TEST_RES(lseek(fd, next_off, SEEK_SET), _ret == next_off);

	for (;;) {
		ssize_t bytes = TEST_RES(syscall(SYS_getdents64, fd, normal_buf,
						 sizeof(normal_buf)),
					 _ret >= 0);
		if (bytes <= 0) {
			break;
		}

		resumed_count = append_dirents(resumed, resumed_count,
					       normal_buf, bytes);
	}

	size_t split_count = prefix_count + resumed_count;
	TEST_RES(split_count, _ret == full_count);

	size_t compare_count = split_count < full_count ? split_count :
							  full_count;
	for (size_t i = 0; i < compare_count; i++) {
		const struct proc_dirent *actual =
			i < prefix_count ? &prefix[i] :
					   &resumed[i - prefix_count];

		TEST_RES(same_visible_dirent(actual, &full_scan[i]), _ret);
	}

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(getdents_proc_continues_when_child_reaped_mid_iteration)
{
	/*
	 * Reaping a still-unemitted `/proc/<pid>` child between small
	 * `getdents64` calls must not make the `/proc` root scan fail with the
	 * child's lookup error.
	 */
	pid_t children[CHILD_COUNT];
	bool child_seen[CHILD_COUNT] = {};
	bool child_reaped[CHILD_COUNT] = {};
	char buf[MID_DIRENT_BUF_SIZE];
	int64_t last_off = 0;
	size_t nonzero_reads = 0;
	size_t reaped_mid_iteration = 0;

	for (size_t i = 0; i < ARRAY_SIZE(children); i++) {
		pid_t pid = fork();
		if (pid == 0) {
			_exit(0);
		}

		TEST_RES(pid, _ret > 0);
		children[i] = pid;
	}

	int fd = TEST_SUCC(open("/proc", O_RDONLY | O_DIRECTORY));

	for (;;) {
		errno = 0;
		ssize_t bytes = syscall(SYS_getdents64, fd, buf, sizeof(buf));
		int saved_errno = errno;

		TEST_RES(saved_errno,
			 bytes >= 0 || (_ret != ENOENT && _ret != ESRCH));
		TEST_RES(bytes, _ret >= 0);
		if (bytes < 0) {
			break;
		}
		if (bytes == 0) {
			break;
		}

		nonzero_reads++;
		bool valid_dirents = true;
		bool monotonic_offsets = true;
		for (size_t pos = 0; pos < (size_t)bytes;) {
			struct linux_dirent64 *dirent =
				(struct linux_dirent64 *)((char *)buf + pos);

			if (dirent->d_reclen == 0 ||
			    pos + dirent->d_reclen > (size_t)bytes) {
				valid_dirents = false;
				break;
			}
			if (dirent->d_off <= last_off) {
				monotonic_offsets = false;
			}
			last_off = dirent->d_off;

			for (size_t i = 0; i < ARRAY_SIZE(children); i++) {
				char name[32];

				snprintf(name, sizeof(name), "%d", children[i]);
				if (strcmp(dirent->d_name, name) == 0) {
					child_seen[i] = true;
					break;
				}
			}

			pos += dirent->d_reclen;
		}
		TEST_RES(valid_dirents, _ret);
		TEST_RES(monotonic_offsets, _ret);

		for (size_t i = 0; i < ARRAY_SIZE(children); i++) {
			if (child_seen[i] || child_reaped[i]) {
				continue;
			}

			TEST_RES(waitpid(children[i], NULL, 0),
				 _ret == children[i]);
			child_reaped[i] = true;
			reaped_mid_iteration++;
			break;
		}
	}

	TEST_RES(nonzero_reads, _ret > 1);
	TEST_RES(reaped_mid_iteration, _ret > 0);

	for (size_t i = 0; i < ARRAY_SIZE(children); i++) {
		if (!child_reaped[i]) {
			TEST_RES(waitpid(children[i], NULL, 0),
				 _ret == children[i]);
		}
	}

	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(close_extra_proc_fds)
{
	for (size_t i = 0; i < ARRAY_SIZE(extra_fds); i++) {
		CHECK(close(extra_fds[i]));
	}
}
END_SETUP()
