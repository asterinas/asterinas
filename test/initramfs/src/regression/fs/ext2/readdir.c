// SPDX-License-Identifier: MPL-2.0

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ext2/readdir_test"

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

static void remove_if_exists(const char *path)
{
	CHECK_WITH(rmdir(path), _ret == 0 || errno == ENOENT);
}

static void unlink_if_exists(const char *path)
{
	CHECK_WITH(unlink(path), _ret == 0 || errno == ENOENT);
}

FN_SETUP(setup_base_dir)
{
	ensure_dir(BASE_DIR);
}
END_SETUP()

FN_TEST(readdir_seekdir_resume)
{
	const char *dir = BASE_DIR "/seekdir_test";
	char path[256];
	const int num_files = 6;

	ensure_dir(dir);
	for (int i = 0; i < num_files; i++) {
		snprintf(path, sizeof(path), "%s/file_%03d", dir, i);
		int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
		TEST_SUCC(close(fd));
	}

	DIR *dp = TEST_SUCC(opendir(dir));

	struct dirent *ent;
	int skip = 3;
	for (int i = 0; i < skip; i++)
		ent = readdir(dp);

	long saved_pos = telldir(dp);

	char first_remaining[256] = { 0 };
	ent = readdir(dp);
	if (ent)
		snprintf(first_remaining, sizeof(first_remaining), "%s",
			 ent->d_name);
	closedir(dp);

	dp = TEST_SUCC(opendir(dir));

	seekdir(dp, saved_pos);
	ent = readdir(dp);
	if (ent)
		TEST_RES(strcmp(ent->d_name, first_remaining), _ret == 0);
	closedir(dp);

	for (int i = num_files - 1; i >= 0; i--) {
		snprintf(path, sizeof(path), "%s/file_%03d", dir, i);
		unlink_if_exists(path);
	}
	remove_if_exists(dir);
}
END_TEST()

FN_TEST(mkdir_contains_dot_dotdot)
{
	const char *dir = BASE_DIR "/dot_dotdot";

	ensure_dir(dir);

	DIR *dp = TEST_SUCC(opendir(dir));

	struct dirent *ent;
	int count = 0;
	int found_dot = 0, found_dotdot = 0;
	while ((ent = readdir(dp)) != NULL) {
		if (strcmp(ent->d_name, ".") == 0)
			found_dot = 1;
		else if (strcmp(ent->d_name, "..") == 0)
			found_dotdot = 1;
		count++;
	}
	closedir(dp);

	TEST_RES(count, _ret == 2);
	TEST_RES(found_dot, _ret == 1);
	TEST_RES(found_dotdot, _ret == 1);

	remove_if_exists(dir);
}
END_TEST()

FN_TEST(dir_growth_many_files)
{
	const char *dir = BASE_DIR "/many_files";
	const int num_files = 200;
	char path[256];

	ensure_dir(dir);

	for (int i = 0; i < num_files; i++) {
		snprintf(path, sizeof(path), "%s/file_%03d", dir, i);
		int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
		TEST_SUCC(close(fd));
	}

	DIR *dp = TEST_SUCC(opendir(dir));

	int count = 0;
	while (readdir(dp) != NULL)
		count++;
	closedir(dp);

	TEST_RES(count, _ret == num_files + 2);

	for (int i = num_files - 1; i >= 0; i--) {
		snprintf(path, sizeof(path), "%s/file_%03d", dir, i);
		unlink_if_exists(path);
	}
	remove_if_exists(dir);
}
END_TEST()

FN_TEST(dot_dotdot_semantics)
{
	const char *parent = BASE_DIR "/parent";
	const char *child = BASE_DIR "/parent/child";
	struct stat st_parent, st_child, st_dot, st_dotdot;
	char dot_path[256], dotdot_path[256];

	ensure_dir(parent);
	ensure_dir(child);

	TEST_SUCC(stat(parent, &st_parent));
	TEST_SUCC(stat(child, &st_child));

	snprintf(dot_path, sizeof(dot_path), "%s/.", child);
	snprintf(dotdot_path, sizeof(dotdot_path), "%s/..", child);

	TEST_RES(stat(dot_path, &st_dot),
		 (ino_t)st_dot.st_ino == (ino_t)st_child.st_ino);
	TEST_RES(stat(dotdot_path, &st_dotdot),
		 (ino_t)st_dotdot.st_ino == (ino_t)st_parent.st_ino);

	remove_if_exists(child);
	remove_if_exists(parent);
}
END_TEST()
