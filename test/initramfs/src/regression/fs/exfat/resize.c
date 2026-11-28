// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

// Regression tests for exFAT file resizing, including issue #4110.
// Set EXFAT_MOUNT_POINT or EXFAT_CLUSTER_SIZE to override the defaults.
// The cluster size is measured from a one-byte allocation when unset.
#define DEFAULT_MOUNT_POINT "/exfat"
#define SECTOR_SIZE 512
#define MAX_CLUSTER_SIZE (1L << 20)

static const char *mount_point;
static long cluster_size;
static off_t huge_offset;

static int format_path(char *path, size_t path_size, const char *name)
{
	return snprintf(path, path_size, "%s/%s", mount_point, name);
}

static long get_cluster_size(void)
{
	const char *cluster_size_env = getenv("EXFAT_CLUSTER_SIZE");
	char *end;
	int parse_error;
	long result;

	if (cluster_size_env != NULL) {
		errno = 0;
		result = strtol(cluster_size_env, &end, 0);
		parse_error = errno;
		CHECK_WITH(result, parse_error == 0 &&
					   end != cluster_size_env &&
					   *end == '\0' && result > 0 &&
					   result % SECTOR_SIZE == 0 &&
					   result <= MAX_CLUSTER_SIZE);
		return result;
	}

	char path[PATH_MAX];
	struct stat st;
	int fd;

	CHECK_WITH(format_path(path, sizeof(path), "resize-setup.XXXXXX"),
		   _ret > 0 && (size_t)_ret < sizeof(path));
	fd = CHECK(mkstemp(path));
	CHECK_WITH(write(fd, "A", 1), _ret == 1);
	CHECK_WITH(fstat(fd, &st), _ret == 0 && st.st_blocks > 0);
	result = st.st_blocks * SECTOR_SIZE;
	CHECK_WITH(result, result > 0 && result <= MAX_CLUSTER_SIZE);
	CHECK(close(fd));
	CHECK(unlink(path));
	return result;
}

FN_SETUP(prepare_exfat_resize)
{
	const char *mount_point_env = getenv("EXFAT_MOUNT_POINT");

	mount_point = mount_point_env != NULL && mount_point_env[0] != '\0' ?
			      mount_point_env :
			      DEFAULT_MOUNT_POINT;
	cluster_size = get_cluster_size();
	huge_offset = (off_t)(((1ULL << 32) + 1) * cluster_size);
}
END_SETUP()

FN_TEST(huge_pwrite_fails_without_changing_file)
{
	char path[PATH_MAX];
	long expected_blocks = cluster_size / SECTOR_SIZE;
	struct stat st;
	char buffer[8] = { 0 };
	int fd;

	TEST_RES(format_path(path, sizeof(path), "resize-pwrite"),
		 _ret > 0 && (size_t)_ret < sizeof(path));
	fd = TEST_SUCC(open(path, O_RDWR | O_CREAT | O_TRUNC, 0644));

	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 0);
	TEST_RES(pwrite(fd, "A", 1, 0), _ret == 1);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 1 &&
					 st.st_blocks == expected_blocks);

	TEST(pwrite(fd, "B", 1, huge_offset), EFBIG, _ret == -1);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 1 &&
					 st.st_blocks == expected_blocks);

	TEST_RES(pread(fd, buffer, 4, 0), _ret == 1 && buffer[0] == 'A');
	TEST_RES(pread(fd, buffer, 4, cluster_size), _ret == 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(huge_ftruncate_fails_without_changing_file)
{
	char path[PATH_MAX];
	struct stat st;
	int fd;

	TEST_RES(format_path(path, sizeof(path), "resize-ftruncate"),
		 _ret > 0 && (size_t)_ret < sizeof(path));
	fd = TEST_SUCC(open(path, O_RDWR | O_CREAT | O_TRUNC, 0644));

	TEST_RES(write(fd, "0123", 4), _ret == 4);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 4);

	TEST(ftruncate(fd, huge_offset), EFBIG, _ret == -1);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 4);
	TEST_SUCC(ftruncate(fd, 4));

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(shrink_takes_effect)
{
	char path[PATH_MAX];
	struct stat st;
	char buffer[32];
	int fd;

	TEST_RES(format_path(path, sizeof(path), "resize-shrink"),
		 _ret > 0 && (size_t)_ret < sizeof(path));
	fd = TEST_SUCC(open(path, O_RDWR | O_CREAT | O_TRUNC, 0644));

	TEST_RES(write(fd, "0123456789abcdef", 16), _ret == 16);
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 16);
	TEST_SUCC(ftruncate(fd, 4));
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 4);

	memset(buffer, 0, sizeof(buffer));
	TEST_RES(pread(fd, buffer, sizeof(buffer), 0),
		 _ret == 4 && memcmp(buffer, "0123", 4) == 0);
	TEST_RES(pread(fd, buffer, 12, 4), _ret == 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(multi_cluster_shrink_takes_effect)
{
	char path[PATH_MAX];
	long expected_blocks = cluster_size / SECTOR_SIZE;
	struct stat st;
	char buffer[8] = { 0 };
	int fd;

	TEST_RES(format_path(path, sizeof(path), "resize-multi-cluster"),
		 _ret > 0 && (size_t)_ret < sizeof(path));
	fd = TEST_SUCC(open(path, O_RDWR | O_CREAT | O_TRUNC, 0644));

	TEST_RES(pwrite(fd, "A", 1, 0), _ret == 1);
	TEST_RES(pwrite(fd, "B", 1, (off_t)(2 * cluster_size)), _ret == 1);
	TEST_RES(fstat(fd, &st),
		 _ret == 0 && st.st_blocks >= 3 * expected_blocks);

	TEST_SUCC(ftruncate(fd, 4));
	TEST_RES(fstat(fd, &st), _ret == 0 && st.st_size == 4);
	TEST_RES(pread(fd, buffer, 4, 0), _ret == 4 && buffer[0] == 'A');

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()
