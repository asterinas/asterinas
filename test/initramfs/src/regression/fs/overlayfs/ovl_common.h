// SPDX-License-Identifier: MPL-2.0

#ifndef OVL_COMMON_H
#define OVL_COMMON_H

/*
 * The shared pieces of the overlayfs regression tests.
 *
 * Every test builds the same shape of tree: a lower, an upper, a work and a
 * merged directory under one per-test base directory, and then mounts the
 * overlay over them. This header owns that layout and the file operations the
 * tests perform on it, so a test file only has to describe what makes its own
 * tree different.
 *
 * A test file defines BASE_DIR before including this header. A test whose lower
 * layer has to live somewhere else, so that the mount is not same-filesystem,
 * defines LOWER_DIR as well.
 */

#include <dirent.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#ifndef BASE_DIR
#error "define BASE_DIR before including ovl_common.h"
#endif

#ifndef LOWER_DIR
#define LOWER_DIR BASE_DIR "/lower"
#endif
#define UPPER_DIR BASE_DIR "/upper"
#define WORK_DIR BASE_DIR "/work"
#define MERGED_DIR BASE_DIR "/merged"

/* Creates `path`, aborting the test if it cannot be created. */
static inline void create_dir(const char *path)
{
	CHECK(mkdir(path, 0755));
}

/* Creates `path` holding `content`, aborting the test if it cannot be written. */
static inline void write_file(const char *path, const char *content)
{
	int fd = CHECK(open(path, O_WRONLY | O_CREAT, 0644));

	CHECK(write(fd, content, strlen(content)));
	CHECK(close(fd));
}

/* Creates the base, upper, work, lower and merged directories of the tree. */
static inline void create_overlay_dirs(void)
{
	create_dir(BASE_DIR);
	create_dir(UPPER_DIR);
	create_dir(WORK_DIR);
	create_dir(LOWER_DIR);
	create_dir(MERGED_DIR);
}

/*
 * Removes `path`, and everything under it when it is a directory. A path that
 * does not exist is not an error, so a cleanup can name a temp that the test
 * never got around to creating. Returns 0 on success and -1 on failure, which
 * lets a caller write `CHECK(remove_tree(...))`.
 */
static inline int remove_tree(const char *path)
{
	struct stat st;

	if (lstat(path, &st) != 0)
		return errno == ENOENT ? 0 : -1;
	if (!S_ISDIR(st.st_mode))
		return unlink(path);

	DIR *dir = opendir(path);
	if (dir == NULL)
		return rmdir(path);

	struct dirent *entry;
	errno = 0;
	while ((entry = readdir(dir)) != NULL) {
		char child[PATH_MAX];
		int ret;

		if (strcmp(entry->d_name, ".") == 0 ||
		    strcmp(entry->d_name, "..") == 0) {
			errno = 0;
			continue;
		}
		ret = snprintf(child, sizeof(child), "%s/%s", path,
			       entry->d_name);
		if (ret < 0 || (size_t)ret >= sizeof(child)) {
			closedir(dir);
			return -1;
		}
		if (remove_tree(child) != 0) {
			closedir(dir);
			return -1;
		}
		errno = 0;
	}
	if (errno != 0) {
		closedir(dir);
		return -1;
	}
	if (closedir(dir) != 0)
		return -1;
	return rmdir(path);
}

/*
 * Removes the merged mount point and the upper and work halves of the tree,
 * the tail that every overlay cleanup shares. A test whose lower layer lives on
 * another filesystem removes its own lower side afterwards.
 */
static inline void remove_overlay_dirs(void)
{
	CHECK(rmdir(MERGED_DIR));
	CHECK(remove_tree(WORK_DIR "/work"));
	CHECK(rmdir(WORK_DIR));
	CHECK(rmdir(UPPER_DIR));
}

/*
 * Mounts the overlay at MERGED_DIR over the three layer directories, with
 * `extra_options` appended to the mount options. `extra_options` is either an
 * empty string or a string that starts with a comma.
 */
static inline void mount_overlay_with(const char *extra_options)
{
	char options[256];

	snprintf(options, sizeof(options),
		 "lowerdir=%s,upperdir=%s,workdir=%s%s", LOWER_DIR, UPPER_DIR,
		 WORK_DIR, extra_options);

	CHECK(mount("overlay", MERGED_DIR, "overlay", 0, options));
}

/* Mounts the overlay with the default options. */
static inline void mount_overlay(void)
{
	mount_overlay_with("");
}

#endif
