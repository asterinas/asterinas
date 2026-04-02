// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <fcntl.h>
#include <stdbool.h>
#include <stdint.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

struct linux_dirent64 {
	uint64_t d_ino;
	int64_t d_off;
	unsigned short d_reclen;
	unsigned char d_type;
	char d_name[];
};

struct parsed_dirent {
	uint64_t inode;
	int64_t next_off;
	char name[256];
};

static bool dirent_id_matches(const struct parsed_dirent *lhs,
			      const struct parsed_dirent *rhs)
{
	return strcmp(lhs->name, rhs->name) == 0 && lhs->inode == rhs->inode;
}

static ssize_t read_dirent_batch(int fd, void *raw_buf, size_t raw_buf_len,
				 struct parsed_dirent *entries,
				 size_t entry_capacity, size_t *entry_count)
{
	ssize_t bytes;
	size_t parsed = 0;
	size_t pos = 0;

	/*
	 * The caller controls the raw buffer size so tests can force short
	 * `getdents64` batches and validate resume behavior across calls.
	 */
	bytes = syscall(SYS_getdents64, fd, raw_buf, raw_buf_len);
	if (bytes <= 0) {
		*entry_count = 0;
		return bytes;
	}

	while (pos < (size_t)bytes) {
		struct linux_dirent64 *raw_dirent =
			(struct linux_dirent64 *)((char *)raw_buf + pos);

		if (parsed >= entry_capacity) {
			errno = EOVERFLOW;
			return -1;
		}
		if (raw_dirent->d_reclen == 0 ||
		    pos + raw_dirent->d_reclen > (size_t)bytes) {
			errno = EIO;
			return -1;
		}

		entries[parsed].inode = raw_dirent->d_ino;
		entries[parsed].next_off = raw_dirent->d_off;
		snprintf(entries[parsed].name, sizeof(entries[parsed].name),
			 "%s", raw_dirent->d_name);

		parsed++;
		pos += raw_dirent->d_reclen;
	}

	*entry_count = parsed;
	return bytes;
}

static ssize_t read_dirents(int fd, struct parsed_dirent *entries,
			    size_t entry_capacity)
{
	char batch_buf[512];
	size_t total = 0;

	/* Keep iterating in bounded batches instead of assuming one large read. */
	for (;;) {
		size_t batch_count = 0;
		ssize_t batch_bytes;

		batch_bytes = read_dirent_batch(
			fd, batch_buf, sizeof(batch_buf), entries + total,
			entry_capacity - total, &batch_count);
		if (batch_bytes <= 0) {
			return batch_bytes < 0 ? -1 : (ssize_t)total;
		}

		total += batch_count;
	}
}

static bool is_dot_or_dotdot(const char *name)
{
	return strcmp(name, ".") == 0 || strcmp(name, "..") == 0;
}

static int dirent_inodes_match_stat(const char *dir_path)
{
	struct parsed_dirent entries[128];
	struct stat stat_buf;
	ssize_t entry_count;
	int dir_fd;
	int ret = 1;
	size_t i;

	dir_fd = open(dir_path, O_RDONLY | O_DIRECTORY);
	if (dir_fd < 0) {
		return -1;
	}

	entry_count = read_dirents(dir_fd, entries,
				   sizeof(entries) / sizeof(entries[0]));
	if (entry_count < 0) {
		ret = -1;
		goto out;
	}

	for (i = 0; i < (size_t)entry_count; i++) {
		if (is_dot_or_dotdot(entries[i].name)) {
			continue;
		}

		if (fstatat(dir_fd, entries[i].name, &stat_buf,
			    AT_SYMLINK_NOFOLLOW) < 0) {
			ret = -1;
			goto out;
		}
		if (entries[i].inode != stat_buf.st_ino) {
			errno = EIO;
			ret = 0;
			goto out;
		}
	}

out:
	(void)close(dir_fd);
	return ret;
}

static int resume_matches_full_scan(
	const struct parsed_dirent *full_scan_entries, size_t full_scan_count,
	const struct parsed_dirent *first_batch_entries,
	size_t first_batch_count, const struct parsed_dirent *resumed_entries,
	size_t resumed_count)
{
	size_t i;

	if (first_batch_count > full_scan_count) {
		errno = EINVAL;
		return -1;
	}
	if (full_scan_count - first_batch_count != resumed_count) {
		errno = EIO;
		return 0;
	}

	for (i = 0; i < first_batch_count; i++) {
		if (!dirent_id_matches(&first_batch_entries[i],
				       &full_scan_entries[i])) {
			errno = EIO;
			return 0;
		}
	}

	for (i = 0; i < resumed_count; i++) {
		const struct parsed_dirent *expected =
			&full_scan_entries[first_batch_count + i];
		const struct parsed_dirent *actual = &resumed_entries[i];

		if (!dirent_id_matches(actual, expected)) {
			errno = EIO;
			return 0;
		}
	}

	return 1;
}

FN_TEST(getdents64_ino_matches_stat_for_proc_self)
{
	TEST_RES(dirent_inodes_match_stat("/proc/self"), _ret == 1);
}
END_TEST()

FN_TEST(getdents64_ino_matches_stat_for_proc_self_task)
{
	TEST_RES(dirent_inodes_match_stat("/proc/self/task"), _ret == 1);
}
END_TEST()

FN_TEST(getdents64_ino_matches_stat_for_proc_self_fd)
{
	int extra_fd_0, extra_fd_1;

	extra_fd_0 = TEST_SUCC(dup(0));
	extra_fd_1 = TEST_SUCC(dup(0));

	TEST_RES(dirent_inodes_match_stat("/proc/self/fd"), _ret == 1);

	TEST_SUCC(close(extra_fd_0));
	TEST_SUCC(close(extra_fd_1));
}
END_TEST()

FN_TEST(getdents64_d_off_resumes_from_next_entry)
{
	struct parsed_dirent first_batch[16];
	struct parsed_dirent full_scan_entries[128];
	struct parsed_dirent resumed_entries[128];
	char first_batch_raw_buf[128];
	size_t first_batch_count = 0;
	ssize_t full_scan_count;
	ssize_t resumed_count;
	int extra_fd_0, extra_fd_1, extra_fd_2;
	int dir_fd;
	int64_t next_off;

	extra_fd_0 = TEST_SUCC(dup(0));
	extra_fd_1 = TEST_SUCC(dup(0));
	extra_fd_2 = TEST_SUCC(dup(0));

	dir_fd = TEST_SUCC(open("/proc/self/fd", O_RDONLY | O_DIRECTORY));

	/*
	 * Keep the first raw buffer intentionally small so the directory is
	 * split into a prefix and a resumable suffix.
	 */
	TEST_RES(read_dirent_batch(dir_fd, first_batch_raw_buf,
				   sizeof(first_batch_raw_buf), first_batch,
				   sizeof(first_batch) / sizeof(first_batch[0]),
				   &first_batch_count),
		 _ret > 0);
	TEST_RES(first_batch_count, _ret > 0);

	next_off = first_batch[first_batch_count - 1].next_off;

	TEST_RES(lseek(dir_fd, 0, SEEK_SET), _ret == 0);
	full_scan_count = TEST_SUCC(read_dirents(
		dir_fd, full_scan_entries,
		sizeof(full_scan_entries) / sizeof(full_scan_entries[0])));

	TEST_RES(lseek(dir_fd, next_off, SEEK_SET), _ret == next_off);
	resumed_count = TEST_SUCC(read_dirents(
		dir_fd, resumed_entries,
		sizeof(resumed_entries) / sizeof(resumed_entries[0])));

	TEST_RES(resume_matches_full_scan(full_scan_entries, full_scan_count,
					  first_batch, first_batch_count,
					  resumed_entries, resumed_count),
		 _ret == 1);

	TEST_SUCC(close(dir_fd));
	TEST_SUCC(close(extra_fd_0));
	TEST_SUCC(close(extra_fd_1));
	TEST_SUCC(close(extra_fd_2));
}
END_TEST()
