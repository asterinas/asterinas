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
#include <unistd.h>

#include "../../common/test.h"

static char base_dir[PATH_MAX];
static char valid_file[PATH_MAX];
static char orig_cwd[PATH_MAX];

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

static void init_name_256(char *name_256)
{
	memset(name_256, 'a', 256);
	name_256[256] = '\0';
}

static void init_name_512(char *name_512)
{
	memset(name_512, 'b', 512);
	name_512[512] = '\0';
}

FN_SETUP(prepare_nametoolong_env)
{
	const char *tmpdir = get_test_tmpdir();
	fprintf(stderr, "fixture=%s\n", tmpdir);

	CHECK_WITH(snprintf(base_dir, sizeof(base_dir), "%s/vfs_nametoolong",
			    tmpdir),
		   _ret > 0 && (size_t)_ret < sizeof(base_dir));
	CHECK_WITH(snprintf(valid_file, sizeof(valid_file), "%s/valid_file",
			    base_dir),
		   _ret > 0 && (size_t)_ret < sizeof(valid_file));

	remove_file_if_exists(valid_file);
	remove_dir_if_exists(base_dir);

	ensure_dir(base_dir);
	int fd = CHECK(creat(valid_file, 0644));
	CHECK(close(fd));

	CHECK_WITH(getcwd(orig_cwd, sizeof(orig_cwd)), _ret != NULL);
	CHECK(chdir(base_dir));
}
END_SETUP()

FN_TEST(creation_name_too_long)
{
	char name_256[257];
	init_name_256(name_256);

	TEST_ERRNO(open(name_256, O_CREAT | O_WRONLY, 0644), ENAMETOOLONG);
	TEST_ERRNO(creat(name_256, 0644), ENAMETOOLONG);
	TEST_ERRNO(mkdir(name_256, 0755), ENAMETOOLONG);
	TEST_ERRNO(mknod(name_256, S_IFREG | 0644, 0), ENAMETOOLONG);
	TEST_ERRNO(symlink("target", name_256), ENAMETOOLONG);
	TEST_ERRNO(link(valid_file, name_256), ENAMETOOLONG);
}
END_TEST()

FN_TEST(removal_name_too_long)
{
	char name_256[257];
	init_name_256(name_256);

	TEST_ERRNO(unlink(name_256), ENAMETOOLONG);
	TEST_ERRNO(rmdir(name_256), ENAMETOOLONG);

	char slash_path[512];
	snprintf(slash_path, sizeof(slash_path), "%s/", name_256);
	TEST_ERRNO(rmdir(slash_path), ENAMETOOLONG);
	TEST_ERRNO(unlink(slash_path), ENAMETOOLONG);
}
END_TEST()

FN_TEST(lookup_and_access_name_too_long)
{
	char name_256[257];
	init_name_256(name_256);

	struct stat st;
	TEST_ERRNO(open(name_256, O_RDONLY), ENAMETOOLONG);
	TEST_ERRNO(stat(name_256, &st), ENAMETOOLONG);
	TEST_ERRNO(lstat(name_256, &st), ENAMETOOLONG);
	TEST_ERRNO(access(name_256, F_OK), ENAMETOOLONG);

	TEST_ERRNO(rename(name_256, valid_file), ENAMETOOLONG);
	TEST_ERRNO(rename(valid_file, name_256), ENAMETOOLONG);
}
END_TEST()

FN_TEST(path_variants_and_dirfd_name_too_long)
{
	char name_256[257];
	char name_512[513];
	init_name_256(name_256);
	init_name_512(name_512);

	char abs_path[PATH_MAX + 512];
	snprintf(abs_path, sizeof(abs_path), "%s/%s", base_dir, name_256);
	TEST_ERRNO(open(abs_path, O_CREAT | O_WRONLY, 0644), ENAMETOOLONG);
	TEST_ERRNO(mkdir(abs_path, 0755), ENAMETOOLONG);
	TEST_ERRNO(unlink(abs_path), ENAMETOOLONG);
	TEST_ERRNO(rmdir(abs_path), ENAMETOOLONG);

	char sub_dir[PATH_MAX + 64];
	snprintf(sub_dir, sizeof(sub_dir), "%s/sub_dir", base_dir);
	remove_dir_if_exists(sub_dir);
	TEST_SUCC(mkdir(sub_dir, 0755));

	char prefixed_path[PATH_MAX + 512];
	snprintf(prefixed_path, sizeof(prefixed_path), "%s/%s", sub_dir,
		 name_256);
	TEST_ERRNO(unlink(prefixed_path), ENAMETOOLONG);
	TEST_ERRNO(rmdir(prefixed_path), ENAMETOOLONG);

	int dirfd = TEST_SUCC(open(sub_dir, O_RDONLY | O_DIRECTORY));
	TEST_ERRNO(openat(dirfd, name_256, O_CREAT | O_WRONLY, 0644),
		   ENAMETOOLONG);
	TEST_ERRNO(mkdirat(dirfd, name_256, 0755), ENAMETOOLONG);
	TEST_ERRNO(unlinkat(dirfd, name_256, 0), ENAMETOOLONG);
	TEST_ERRNO(unlinkat(dirfd, name_256, AT_REMOVEDIR), ENAMETOOLONG);
	TEST_SUCC(close(dirfd));

	TEST_SUCC(rmdir(sub_dir));

	char inter_path[PATH_MAX + 512];
	snprintf(inter_path, sizeof(inter_path), "%s/%s/child", base_dir,
		 name_256);
	TEST_ERRNO(open(inter_path, O_RDONLY), ENAMETOOLONG);
	TEST_ERRNO(unlink(inter_path), ENAMETOOLONG);
	TEST_ERRNO(rmdir(inter_path), ENAMETOOLONG);

	char large_path[PATH_MAX + 1024];
	snprintf(large_path, sizeof(large_path), "%s/%s", base_dir, name_512);
	TEST_ERRNO(open(large_path, O_CREAT | O_WRONLY, 0644), ENAMETOOLONG);
	TEST_ERRNO(unlink(large_path), ENAMETOOLONG);
	TEST_ERRNO(rmdir(large_path), ENAMETOOLONG);
}
END_TEST()

FN_TEST(boundary_name_max)
{
	char name_255[256];
	char name_255_b[256];
	memset(name_255, 'c', 255);
	name_255[255] = '\0';
	memset(name_255_b, 'd', 255);
	name_255_b[255] = '\0';

	int fd = TEST_SUCC(creat(name_255, 0644));
	TEST_SUCC(close(fd));

	struct stat st;
	TEST_SUCC(stat(name_255, &st));
	int fd_ro = TEST_SUCC(open(name_255, O_RDONLY));
	TEST_SUCC(close(fd_ro));

	TEST_SUCC(rename(name_255, name_255_b));
	TEST_SUCC(unlink(name_255_b));

	TEST_SUCC(mkdir(name_255, 0755));
	TEST_SUCC(stat(name_255, &st));
	TEST_SUCC(rename(name_255, name_255_b));
	TEST_SUCC(rmdir(name_255_b));
}
END_TEST()

FN_SETUP(cleanup_nametoolong_env)
{
	if (orig_cwd[0] != '\0') {
		CHECK(chdir(orig_cwd));
	}
	remove_file_if_exists(valid_file);

	char sub_dir[PATH_MAX];
	CHECK_WITH(snprintf(sub_dir, sizeof(sub_dir), "%s/sub_dir", base_dir),
		   _ret > 0 && (size_t)_ret < sizeof(sub_dir));
	remove_dir_if_exists(sub_dir);

	remove_dir_if_exists(base_dir);
}
END_SETUP()
