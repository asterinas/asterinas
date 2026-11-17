// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/vfs.h>
#include <unistd.h>

#include "../../common/test.h"

static char base_dir[PATH_MAX];
static char valid_file[PATH_MAX];
static char orig_cwd[PATH_MAX];

static size_t name_max;
static char name_valid[PATH_MAX];
static char name_valid2[PATH_MAX];
static char name_too_long[PATH_MAX];
static char name_large[PATH_MAX * 2];

static const char *get_test_tmpdir(void)
{
	const char *dir = getenv("TEST_TMPDIR");
	return dir ? dir : "/tmp";
}

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

static void remove_file_if_exists(const char *path)
{
	CHECK_WITH(unlink(path), _ret >= 0 || errno == ENOENT);
}

static void remove_dir_if_exists(const char *path)
{
	CHECK_WITH(rmdir(path), _ret >= 0 || errno == ENOENT);
}

FN_SETUP(prepare_name_max_env)
{
	const char *tmpdir = get_test_tmpdir();
	fprintf(stderr, "fixture=%s\n", tmpdir);

	CHECK_WITH(snprintf(base_dir, sizeof(base_dir), "%s/name_max", tmpdir),
		   _ret > 0 && (size_t)_ret < sizeof(base_dir));
	CHECK_WITH(snprintf(valid_file, sizeof(valid_file), "%s/valid_file",
			    base_dir),
		   _ret > 0 && (size_t)_ret < sizeof(valid_file));

	remove_file_if_exists(valid_file);
	remove_dir_if_exists(base_dir);

	ensure_dir(base_dir);
	int fd = CHECK(creat(valid_file, 0644));
	CHECK(close(fd));

	struct statfs st;
	CHECK_WITH(statfs(base_dir, &st), _ret == 0);
	name_max = (size_t)st.f_namelen;
	CHECK_WITH(name_max, name_max > 0 && name_max < PATH_MAX);

	memset(name_valid, 'a', name_max);
	name_valid[name_max] = '\0';

	memset(name_valid2, 'b', name_max);
	name_valid2[name_max] = '\0';

	memset(name_too_long, 'c', name_max + 1);
	name_too_long[name_max + 1] = '\0';

	memset(name_large, 'd', name_max + 256);
	name_large[name_max + 256] = '\0';

	CHECK_WITH(getcwd(orig_cwd, sizeof(orig_cwd)), _ret != NULL);
	CHECK(chdir(base_dir));
}
END_SETUP()

FN_TEST(creation_name_too_long)
{
	TEST_ERRNO(open(name_too_long, O_CREAT | O_WRONLY, 0644), ENAMETOOLONG);
	TEST_ERRNO(creat(name_too_long, 0644), ENAMETOOLONG);
	TEST_ERRNO(mkdir(name_too_long, 0755), ENAMETOOLONG);
	TEST_ERRNO(mknod(name_too_long, S_IFREG | 0644, 0), ENAMETOOLONG);
	TEST_ERRNO(symlink("target", name_too_long), ENAMETOOLONG);
	TEST_ERRNO(link(valid_file, name_too_long), ENAMETOOLONG);
}
END_TEST()

FN_TEST(removal_name_too_long)
{
	TEST_ERRNO(unlink(name_too_long), ENAMETOOLONG);
	TEST_ERRNO(rmdir(name_too_long), ENAMETOOLONG);

	char slash_path[PATH_MAX + 16];
	CHECK_WITH(snprintf(slash_path, sizeof(slash_path), "%s/",
			    name_too_long),
		   _ret > 0 && (size_t)_ret < sizeof(slash_path));
	TEST_ERRNO(rmdir(slash_path), ENAMETOOLONG);
	TEST_ERRNO(unlink(slash_path), ENAMETOOLONG);
}
END_TEST()

FN_TEST(lookup_and_access_name_too_long)
{
	struct stat st;
	TEST_ERRNO(open(name_too_long, O_RDONLY), ENAMETOOLONG);
	TEST_ERRNO(stat(name_too_long, &st), ENAMETOOLONG);
	TEST_ERRNO(lstat(name_too_long, &st), ENAMETOOLONG);
	TEST_ERRNO(access(name_too_long, F_OK), ENAMETOOLONG);

	TEST_ERRNO(rename(name_too_long, valid_file), ENAMETOOLONG);
	TEST_ERRNO(rename(valid_file, name_too_long), ENAMETOOLONG);
}
END_TEST()

FN_TEST(path_variants_and_dirfd_name_too_long)
{
	char abs_path[PATH_MAX + 512];
	CHECK_WITH(snprintf(abs_path, sizeof(abs_path), "%s/%s", base_dir,
			    name_too_long),
		   _ret > 0 && (size_t)_ret < sizeof(abs_path));
	TEST_ERRNO(open(abs_path, O_CREAT | O_WRONLY, 0644), ENAMETOOLONG);
	TEST_ERRNO(mkdir(abs_path, 0755), ENAMETOOLONG);
	TEST_ERRNO(unlink(abs_path), ENAMETOOLONG);
	TEST_ERRNO(rmdir(abs_path), ENAMETOOLONG);

	char sub_dir[PATH_MAX + 64];
	CHECK_WITH(snprintf(sub_dir, sizeof(sub_dir), "%s/sub_dir", base_dir),
		   _ret > 0 && (size_t)_ret < sizeof(sub_dir));
	remove_dir_if_exists(sub_dir);
	TEST_SUCC(mkdir(sub_dir, 0755));

	char prefixed_path[PATH_MAX + 512];
	CHECK_WITH(snprintf(prefixed_path, sizeof(prefixed_path), "%s/%s",
			    sub_dir, name_too_long),
		   _ret > 0 && (size_t)_ret < sizeof(prefixed_path));
	TEST_ERRNO(unlink(prefixed_path), ENAMETOOLONG);
	TEST_ERRNO(rmdir(prefixed_path), ENAMETOOLONG);

	int dirfd = TEST_SUCC(open(sub_dir, O_RDONLY | O_DIRECTORY));
	TEST_ERRNO(openat(dirfd, name_too_long, O_CREAT | O_WRONLY, 0644),
		   ENAMETOOLONG);
	TEST_ERRNO(mkdirat(dirfd, name_too_long, 0755), ENAMETOOLONG);
	TEST_ERRNO(unlinkat(dirfd, name_too_long, 0), ENAMETOOLONG);
	TEST_ERRNO(unlinkat(dirfd, name_too_long, AT_REMOVEDIR), ENAMETOOLONG);
	TEST_SUCC(close(dirfd));

	TEST_SUCC(rmdir(sub_dir));

	char inter_path[PATH_MAX + 512];
	CHECK_WITH(snprintf(inter_path, sizeof(inter_path), "%s/%s/child",
			    base_dir, name_too_long),
		   _ret > 0 && (size_t)_ret < sizeof(inter_path));
	TEST_ERRNO(open(inter_path, O_RDONLY), ENAMETOOLONG);
	TEST_ERRNO(unlink(inter_path), ENAMETOOLONG);
	TEST_ERRNO(rmdir(inter_path), ENAMETOOLONG);

	char large_path[PATH_MAX + 1024];
	CHECK_WITH(snprintf(large_path, sizeof(large_path), "%s/%s", base_dir,
			    name_large),
		   _ret > 0 && (size_t)_ret < sizeof(large_path));
	TEST_ERRNO(open(large_path, O_CREAT | O_WRONLY, 0644), ENAMETOOLONG);
	TEST_ERRNO(unlink(large_path), ENAMETOOLONG);
	TEST_ERRNO(rmdir(large_path), ENAMETOOLONG);
}
END_TEST()

FN_TEST(boundary_name_max)
{
	int fd = TEST_SUCC(creat(name_valid, 0644));
	TEST_SUCC(close(fd));

	struct stat st;
	TEST_SUCC(stat(name_valid, &st));
	int fd_ro = TEST_SUCC(open(name_valid, O_RDONLY));
	TEST_SUCC(close(fd_ro));

	TEST_SUCC(rename(name_valid, name_valid2));
	TEST_SUCC(unlink(name_valid2));

	TEST_SUCC(mkdir(name_valid, 0755));
	TEST_SUCC(stat(name_valid, &st));
	TEST_SUCC(rename(name_valid, name_valid2));
	TEST_SUCC(rmdir(name_valid2));
}
END_TEST()

FN_SETUP(cleanup_name_max_env)
{
	if (orig_cwd[0] != '\0') {
		CHECK(chdir(orig_cwd));
	}
	remove_file_if_exists(valid_file);

	char sub_dir[PATH_MAX + 64];
	CHECK_WITH(snprintf(sub_dir, sizeof(sub_dir), "%s/sub_dir", base_dir),
		   _ret > 0 && (size_t)_ret < sizeof(sub_dir));
	remove_dir_if_exists(sub_dir);

	remove_dir_if_exists(base_dir);
}
END_SETUP()
