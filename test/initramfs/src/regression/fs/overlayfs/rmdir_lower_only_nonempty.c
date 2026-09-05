// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <dirent.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ovl_rmdir_lower_only"
#define UPPER_DIR BASE_DIR "/upper"
#define WORK_DIR BASE_DIR "/work"
#define LOWER_DIR BASE_DIR "/lower"
#define MERGED_DIR BASE_DIR "/merged"

static void create_dir(const char *path)
{
	CHECK(mkdir(path, 0755));
}

static void write_file(const char *path)
{
	int fd = CHECK(open(path, O_WRONLY | O_CREAT, 0644));

	CHECK(write(fd, "data", 4));
	CHECK(close(fd));
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

	create_dir(LOWER_DIR "/target");
	write_file(LOWER_DIR "/target/.wh.foo");
}

static void cleanup_overlay_tree(void)
{
	CHECK(unlink(LOWER_DIR "/target/.wh.foo"));
	CHECK(rmdir(LOWER_DIR "/target"));

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

FN_TEST(rmdir_lower_only_nonempty)
{
	TEST_ERRNO(rmdir(MERGED_DIR "/target"), ENOTEMPTY);
}

END_TEST()

FN_SETUP(cleanup)
{
	CHECK(umount(MERGED_DIR));
	cleanup_overlay_tree();
}

END_SETUP()
