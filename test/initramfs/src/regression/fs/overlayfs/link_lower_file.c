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

#define BASE_DIR "/ovl_link_lower_file"
#define UPPER_DIR BASE_DIR "/upper"
#define WORK_DIR BASE_DIR "/work"
#define LOWER_DIR BASE_DIR "/lower"
#define MERGED_DIR BASE_DIR "/merged"

#define SOURCE_NAME "source"
#define TARGET_DIR_NAME "target"
#define LINK_NAME "link"

static void create_dir(const char *path)
{
	CHECK(mkdir(path, 0755));
}

static void write_file(const char *path)
{
	int fd = CHECK(open(path, O_WRONLY | O_CREAT, 0644));

	CHECK(write(fd, "link-source-data", 16));
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

	write_file(LOWER_DIR "/" SOURCE_NAME);
	create_dir(UPPER_DIR "/" TARGET_DIR_NAME);
}

static void cleanup_overlay_tree(void)
{
	CHECK_WITH(unlink(UPPER_DIR "/" TARGET_DIR_NAME "/" LINK_NAME),
		   _ret == 0 || errno == ENOENT);
	CHECK(rmdir(UPPER_DIR "/" TARGET_DIR_NAME));
	CHECK_WITH(unlink(UPPER_DIR "/" SOURCE_NAME),
		   _ret == 0 || errno == ENOENT);
	CHECK(unlink(LOWER_DIR "/" SOURCE_NAME));

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

FN_TEST(link_lower_file)
{
	struct stat st_source, st_link;

	TEST_SUCC(link(MERGED_DIR "/" SOURCE_NAME,
		       MERGED_DIR "/" TARGET_DIR_NAME "/" LINK_NAME));
	TEST_SUCC(stat(MERGED_DIR "/" SOURCE_NAME, &st_source));
	TEST_SUCC(stat(MERGED_DIR "/" TARGET_DIR_NAME "/" LINK_NAME, &st_link));
	TEST_RES(st_source.st_dev, _ret == st_link.st_dev);
	TEST_RES(st_source.st_ino, _ret == st_link.st_ino);
}

END_TEST()

FN_SETUP(cleanup)
{
	CHECK(umount(MERGED_DIR));
	cleanup_overlay_tree();
}

END_SETUP()
