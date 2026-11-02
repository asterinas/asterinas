// SPDX-License-Identifier: MPL-2.0

// Verifies that data written by the F2FS regression test survives an unmount
// and a fresh mount. It runs after `run_test.sh` has remounted /f2fs.

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/statfs.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/f2fs"
#define IO_BUF_SIZE (64 * 1024)

static int verify_pattern(const char *path, size_t size, unsigned mul,
			  unsigned add, unsigned mod)
{
	int fd = open(path, O_RDONLY);
	if (fd < 0) {
		fprintf(stderr, "open %s: %s\n", path, strerror(errno));
		return -1;
	}

	unsigned char *buf = malloc(IO_BUF_SIZE);
	if (!buf) {
		close(fd);
		return -1;
	}

	int ret = 0;
	size_t off = 0;
	while (off < size) {
		size_t want = size - off;
		if (want > IO_BUF_SIZE)
			want = IO_BUF_SIZE;

		ssize_t n = read(fd, buf, want);
		if (n <= 0) {
			fprintf(stderr, "%s: read at %zu returned %zd\n", path,
				off, n);
			ret = -1;
			break;
		}
		for (ssize_t i = 0; i < n; i++) {
			unsigned expect =
				(unsigned)((off + (size_t)i) * mul + add) % mod;
			if (buf[i] != expect) {
				fprintf(stderr,
					"%s: byte %zu is %u, expected %u\n",
					path, off + (size_t)i, buf[i], expect);
				ret = -1;
				break;
			}
		}
		if (ret)
			break;
		off += (size_t)n;
	}

	free(buf);
	close(fd);
	return ret;
}

static int verify_content(const char *path, const char *expected)
{
	size_t len = strlen(expected);
	char *buf = malloc(len + 1);
	if (!buf)
		return -1;

	int fd = open(path, O_RDONLY);
	if (fd < 0) {
		free(buf);
		return -1;
	}

	size_t done = 0;
	while (done < len) {
		ssize_t n = read(fd, buf + done, len - done);
		if (n <= 0) {
			free(buf);
			close(fd);
			return -1;
		}
		done += (size_t)n;
	}

	char extra;
	ssize_t n = read(fd, &extra, 1);
	int ret = (n == 0 && memcmp(buf, expected, len) == 0) ? 0 : -1;
	free(buf);
	close(fd);
	return ret;
}

FN_SETUP(f2fs_is_mounted)
{
	struct stat st;
	CHECK(stat(BASE_DIR, &st));
	CHECK_WITH(S_ISDIR(st.st_mode), _ret != 0);
}
END_SETUP()

FN_TEST(persisted_files)
{
	struct stat st;

	/* Files and directories renamed before the remount. */
	TEST_RES(verify_content(BASE_DIR "/persist_renamed", "persist-a\n"),
		 _ret == 0);
	TEST_RES(verify_content(BASE_DIR "/persist_moved_dir/keep.txt",
				"keep\n"),
		 _ret == 0);
	TEST_ERRNO(access(BASE_DIR "/persist_a", F_OK), ENOENT);
	TEST_ERRNO(access(BASE_DIR "/persist_dir", F_OK), ENOENT);

	/* Multi-level directories and their files. */
	TEST_RES(verify_content(BASE_DIR "/ml1/ml2/ml3/ml4/f4.txt", "level4\n"),
		 _ret == 0);
	struct stat ml;
	TEST_SUCC(stat(BASE_DIR "/ml1/ml2/ml3/ml4", &ml));
	TEST_RES(S_ISDIR(ml.st_mode), _ret != 0);
	TEST_SUCC(stat(BASE_DIR "/ml1/ml2/ml3/ml4/..", &ml));
	TEST_RES(S_ISDIR(ml.st_mode), _ret != 0);

	/* A file deleted before the remount must stay deleted. */
	TEST_ERRNO(access(BASE_DIR "/persist_deleted", F_OK), ENOENT);

	/* The truncated file must have survived as an empty file. */
	TEST_SUCC(stat(BASE_DIR "/crud_a", &st));
	TEST_RES(st.st_size, _ret == 0);

	/* Deleted directory entries must stay deleted. */
	TEST_ERRNO(access(BASE_DIR "/crud_dir", F_OK), ENOENT);

	/* A large, node-mapped file must still read back correctly. */
	const size_t big_size = 4 * 1024 * 1024 + 4096;
	TEST_SUCC(stat(BASE_DIR "/crud_big", &st));
	TEST_RES(st.st_size, _ret == (off_t)big_size);
	TEST_RES(verify_pattern(BASE_DIR "/crud_big", big_size, 31, 7, 251),
		 _ret == 0);

	/* The sparse file keeps its hole and its tail. */
	TEST_SUCC(stat(BASE_DIR "/crud_t", &st));
	TEST_RES(st.st_size, _ret == 8196);
	char tail[4];
	int fd = TEST_SUCC(open(BASE_DIR "/crud_t", O_RDONLY));
	TEST_RES(pread(fd, tail, sizeof(tail), 8192),
		 _ret == (ssize_t)sizeof(tail));
	TEST_RES(memcmp(tail, "tail", sizeof(tail)), _ret == 0);
	TEST_SUCC(close(fd));

	/* Files from the original image are untouched. */
	TEST_RES(verify_content(BASE_DIR "/hello.txt", "hello f2fs\n"),
		 _ret == 0);
}
END_TEST()

FN_TEST(mount_is_f2fs)
{
	struct statfs st;
	TEST_SUCC(statfs(BASE_DIR, &st));
	TEST_RES(st.f_type, _ret == 0xF2F52010);

	char buf[4096];
	int fd = TEST_SUCC(open("/proc/mounts", O_RDONLY));
	ssize_t len = TEST_SUCC(read(fd, buf, sizeof(buf) - 1));
	TEST_SUCC(close(fd));
	buf[len] = '\0';
	TEST_RES(strstr(buf, "/f2fs f2fs") != NULL, _ret != 0);

	DIR *dir = TEST_SUCC(opendir(BASE_DIR));
	struct dirent *ent;
	int found = 0;
	while ((ent = readdir(dir)) != NULL) {
		if (strcmp(ent->d_name, "persist_renamed") == 0)
			found = 1;
	}
	TEST_SUCC(closedir(dir));
	TEST_RES(found, _ret == 1);
}
END_TEST()
