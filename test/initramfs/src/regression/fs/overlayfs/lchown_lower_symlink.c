// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <dirent.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ovl_lchown_lower_symlink"
#define UPPER_DIR BASE_DIR "/upper"
#define WORK_DIR BASE_DIR "/work"
#define LOWER_DIR BASE_DIR "/lower"
#define MERGED_DIR BASE_DIR "/merged"

#define LINK_NAME "link"
#define LINK_TARGET "lower-symlink-target"
#define NEW_UID 1234
#define NEW_GID 1234

static void create_dir(const char *path)
{
	CHECK(mkdir(path, 0755));
}

static void remove_tree(const char *path)
{
	struct stat st;

	if (lstat(path, &st) != 0)
		return;
	if (!S_ISDIR(st.st_mode)) {
		unlink(path);
		return;
	}

	DIR *dir = opendir(path);
	if (dir == NULL) {
		rmdir(path);
		return;
	}

	struct dirent *entry;
	while ((entry = readdir(dir)) != NULL) {
		char child[PATH_MAX];
		int ret;

		if (strcmp(entry->d_name, ".") == 0 ||
		    strcmp(entry->d_name, "..") == 0)
			continue;
		ret = snprintf(child, sizeof(child), "%s/%s", path,
			       entry->d_name);
		if (ret < 0 || (size_t)ret >= sizeof(child))
			continue;
		remove_tree(child);
	}

	closedir(dir);
	rmdir(path);
}

static void setup_overlay_tree(void)
{
	create_dir(BASE_DIR);
	create_dir(UPPER_DIR);
	create_dir(WORK_DIR);
	create_dir(LOWER_DIR);
	create_dir(MERGED_DIR);

	CHECK(symlink(LINK_TARGET, LOWER_DIR "/" LINK_NAME));
}

static void cleanup_overlay_tree(void)
{
	CHECK_WITH(unlink(UPPER_DIR "/" LINK_NAME),
		   _ret == 0 || errno == ENOENT);
	CHECK(unlink(LOWER_DIR "/" LINK_NAME));

	CHECK(rmdir(MERGED_DIR));
	remove_tree(WORK_DIR "/work");
	CHECK(rmdir(WORK_DIR));
	CHECK(rmdir(UPPER_DIR));
	CHECK(rmdir(LOWER_DIR));
	CHECK(rmdir(BASE_DIR));
}

static void mount_overlay(void)
{
	char options[256];

	snprintf(options, sizeof(options), "lowerdir=%s,upperdir=%s,workdir=%s",
		 LOWER_DIR, UPPER_DIR, WORK_DIR);

	CHECK(mount("overlay", MERGED_DIR, "overlay", 0, options));
}

FN_SETUP(init)
{
	setup_overlay_tree();
	mount_overlay();
}

END_SETUP()

FN_TEST(lchown_lower_symlink)
{
	char target[64] = { 0 };
	struct stat st;

	TEST_SUCC(lchown(MERGED_DIR "/" LINK_NAME, NEW_UID, NEW_GID));
	TEST_SUCC(lstat(MERGED_DIR "/" LINK_NAME, &st));
	TEST_RES(st.st_uid, _ret == NEW_UID);
	TEST_RES(st.st_gid, _ret == NEW_GID);
	TEST_RES(readlink(MERGED_DIR "/" LINK_NAME, target, sizeof(target)),
		 _ret == (ssize_t)strlen(LINK_TARGET));
	TEST_RES(strcmp(target, LINK_TARGET), _ret == 0);
}

END_TEST()

FN_SETUP(cleanup)
{
	CHECK(umount(MERGED_DIR));
	cleanup_overlay_tree();
}

END_SETUP()
