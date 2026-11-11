// SPDX-License-Identifier: MPL-2.0

#ifndef OVL_MOUNTS_H
#define OVL_MOUNTS_H

/*
 * The shared pieces of the overlayfs regression tests that deal with more
 * than one overlay per test binary, the on-disk xattr vocabulary the fixtures
 * fabricate, and the tree builders that match the multi-lower mount helpers.
 *
 * `ovl_common.h` owns the single-tree layout keyed on the compile-time
 * `BASE_DIR`; this header owns the same layout keyed on a run-time `root`
 * (one test binary mounts several independent overlays under one
 * `BASE_DIR`), plus the observation probes such tests share. The two
 * headers coexist: nothing here rewrites or replaces `ovl_common.h`.
 */

#include <dirent.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/xattr.h>
#include <unistd.h>

#include "../../common/test.h"

/* Writes `left`/`right` into `out`, aborting the test on truncation. */
static inline void join_path(char *out, size_t size, const char *left,
			     const char *right)
{
	int ret = snprintf(out, size, "%s/%s", left, right);

	CHECK_WITH(ret, _ret >= 0 && (size_t)_ret < size);
}

/* Creates `root` and its lower, upper, work and merged directories. */
static inline void create_overlay_tree_at(const char *root)
{
	char path[PATH_MAX];

	create_dir(root);
	join_path(path, sizeof(path), root, "lower");
	create_dir(path);
	join_path(path, sizeof(path), root, "upper");
	create_dir(path);
	join_path(path, sizeof(path), root, "work");
	create_dir(path);
	join_path(path, sizeof(path), root, "merged");
	create_dir(path);
}

/*
 * Mounts the overlay at `root`/merged over `root`/{lower,upper,work}, with
 * `extra_options` appended; `extra_options` is either empty or starts with
 * a comma.
 */
static inline void mount_overlay_at(const char *root, const char *extra_options)
{
	char lower[PATH_MAX], upper[PATH_MAX], work[PATH_MAX];
	char merged[PATH_MAX], options[3 * PATH_MAX + 64];
	int ret;

	join_path(lower, sizeof(lower), root, "lower");
	join_path(upper, sizeof(upper), root, "upper");
	join_path(work, sizeof(work), root, "work");
	join_path(merged, sizeof(merged), root, "merged");
	ret = snprintf(options, sizeof(options),
		       "lowerdir=%s,upperdir=%s,workdir=%s%s", lower, upper,
		       work, extra_options);
	CHECK_WITH(ret, _ret >= 0 && (size_t)_ret < sizeof(options));
	CHECK(mount("overlay", merged, "overlay", 0, options));
}

/*
 * Mounts the overlay like `mount_overlay_at`, but over two lower layers
 * `root`/top (upper of the two) and `root`/bottom.
 */
static inline void mount_overlay_multi_lower_at(const char *root,
						const char *extra_options)
{
	char top[PATH_MAX], bottom[PATH_MAX], upper[PATH_MAX], work[PATH_MAX];
	char merged[PATH_MAX], options[4 * PATH_MAX + 64];
	int ret;

	join_path(top, sizeof(top), root, "top");
	join_path(bottom, sizeof(bottom), root, "bottom");
	join_path(upper, sizeof(upper), root, "upper");
	join_path(work, sizeof(work), root, "work");
	join_path(merged, sizeof(merged), root, "merged");
	ret = snprintf(options, sizeof(options),
		       "lowerdir=%s:%s,upperdir=%s,workdir=%s%s", top, bottom,
		       upper, work, extra_options);
	CHECK_WITH(ret, _ret >= 0 && (size_t)_ret < sizeof(options));
	CHECK(mount("overlay", merged, "overlay", 0, options));
}

/*
 * Mounts the overlay like `mount_overlay_multi_lower_at`, but over three
 * lower layers `root`/t1 (the uppermost of the three), `root`/t2 and
 * `root`/bottom.
 */
static inline void mount_overlay_three_lower_at(const char *root,
						const char *extra_options)
{
	char t1[PATH_MAX], t2[PATH_MAX], bottom[PATH_MAX];
	char upper[PATH_MAX], work[PATH_MAX], merged[PATH_MAX];
	char options[6 * PATH_MAX + 64];
	int ret;

	join_path(t1, sizeof(t1), root, "t1");
	join_path(t2, sizeof(t2), root, "t2");
	join_path(bottom, sizeof(bottom), root, "bottom");
	join_path(upper, sizeof(upper), root, "upper");
	join_path(work, sizeof(work), root, "work");
	join_path(merged, sizeof(merged), root, "merged");
	ret = snprintf(options, sizeof(options),
		       "lowerdir=%s:%s:%s,upperdir=%s,workdir=%s%s", t1, t2,
		       bottom, upper, work, extra_options);
	CHECK_WITH(ret, _ret >= 0 && (size_t)_ret < sizeof(options));
	CHECK(mount("overlay", merged, "overlay", 0, options));
}

/*
 * Creates `root` with the two lower layers `mount_overlay_multi_lower_at`
 * reads.
 */
static inline void create_multi_lower_tree_at(const char *root)
{
	char path[PATH_MAX];

	create_overlay_tree_at(root);
	join_path(path, sizeof(path), root, "top");
	create_dir(path);
	join_path(path, sizeof(path), root, "bottom");
	create_dir(path);
}

/*
 * Creates `root` with the three lower layers `mount_overlay_three_lower_at`
 * reads.
 */
static inline void create_three_lower_tree_at(const char *root)
{
	char path[PATH_MAX];

	create_overlay_tree_at(root);
	join_path(path, sizeof(path), root, "t1");
	create_dir(path);
	join_path(path, sizeof(path), root, "t2");
	create_dir(path);
	join_path(path, sizeof(path), root, "bottom");
	create_dir(path);
}

/* Unmounts the overlay mounted at `root`/merged. */
static inline void unmount_overlay_at(const char *root)
{
	char merged[PATH_MAX];

	join_path(merged, sizeof(merged), root, "merged");
	CHECK(umount(merged));
}

/*
 * Reads the whole body of the file at `path` into `buf` as a
 * NUL-terminated string, truncating if it does not fit. Returns the number
 * of bytes read, or -1 when `size` is zero or the file cannot be opened;
 * on failure `buf` becomes the empty string and `errno` is cleared, so the
 * result can be checked with `TEST_RES`.
 */
static inline ssize_t read_body(const char *path, char *buf, size_t size)
{
	int fd = open(path, O_RDONLY);
	size_t used = 0;

	if (size == 0)
		return -1;
	buf[0] = '\0';
	if (fd < 0) {
		errno = 0;
		return -1;
	}
	while (used + 1 < size) {
		ssize_t ret = read(fd, buf + used, size - used - 1);

		if (ret <= 0)
			break;
		used += (size_t)ret;
	}
	buf[used] = '\0';
	close(fd);
	errno = 0;
	return (ssize_t)used;
}

/*
 * Returns 1 when `dir_path` holds an entry named `name` besides "." and
 * "..", 0 when it does not, and -1 on a read error. `errno` is cleared
 * before returning.
 */
static inline int find_entry(const char *dir_path, const char *name)
{
	DIR *dir = opendir(dir_path);
	struct dirent *entry;
	int found = 0;

	if (dir == NULL)
		return -1;
	for (;;) {
		errno = 0;
		entry = readdir(dir);
		if (entry == NULL)
			break;
		if (strcmp(entry->d_name, ".") == 0 ||
		    strcmp(entry->d_name, "..") == 0)
			continue;
		if (strcmp(entry->d_name, name) == 0) {
			found = 1;
			break;
		}
	}
	if (errno != 0)
		found = -1;
	closedir(dir);
	return found;
}

/*
 * Returns the number of entries in `dir_path` besides "." and "..", or -1
 * on a read error. `errno` is cleared before returning.
 */
static inline int count_entries(const char *dir_path)
{
	DIR *dir = opendir(dir_path);
	struct dirent *entry;
	int count = 0;

	if (dir == NULL)
		return -1;
	for (;;) {
		errno = 0;
		entry = readdir(dir);
		if (entry == NULL)
			break;
		if (strcmp(entry->d_name, ".") == 0 ||
		    strcmp(entry->d_name, "..") == 0)
			continue;
		count++;
	}
	if (errno != 0)
		count = -1;
	closedir(dir);
	return count;
}

/* Writes `value` as extended attribute `name` of `path`, aborting on failure. */
static inline void set_xattr_value(const char *path, const char *name,
				   const void *value, size_t size)
{
	CHECK(setxattr(path, name, value, size, 0));
}

/*
 * Returns 1 when extended attribute `name` of `path` equals the `size`
 * bytes at `expected`, 0 otherwise. `errno` is cleared before returning,
 * so the result can be checked with `TEST_RES`.
 */
static inline int xattr_matches(const char *path, const char *name,
				const void *expected, size_t size)
{
	char value[256];
	ssize_t written = getxattr(path, name, value, sizeof(value));
	int matches = written == (ssize_t)size &&
		      memcmp(value, expected, size) == 0;

	errno = 0;
	return matches;
}

/*
 * Returns 1 when `path` has no extended attribute `name` (the read fails
 * with `ENODATA`), 0 otherwise. `errno` is cleared before returning, so
 * the result can be checked with `TEST_RES`.
 */
static inline int xattr_absent(const char *path, const char *name)
{
	char value[64];
	int absent = getxattr(path, name, value, sizeof(value)) == -1 &&
		     errno == ENODATA;

	errno = 0;
	return absent;
}

#define METACOPY_XATTR "trusted.overlay.metacopy"
#define REDIRECT_XATTR "trusted.overlay.redirect"
#define OPAQUE_XATTR "trusted.overlay.opaque"

/*
 * The v0 metacopy record: version 0, length 4 (the header alone), no flags and
 * no digest algorithm - Linux's `OVL_METACOPY_INIT`. Linux's own writer stores
 * the same record as a zero-length value; both readers accept both forms.
 *
 * Reference: `fs/overlayfs/util.c:1277-1287` and `overlayfs.h:165-175`
 * (Linux v6.18); the reader rules are in `inode/copyup/metacopy.rs`.
 */
static const char metacopy_v0[4] = { 0, 4, 0, 0 };

/* Writes the v0 metacopy record on `path`. */
static inline void set_metacopy(const char *path)
{
	set_xattr_value(path, METACOPY_XATTR, metacopy_v0, sizeof(metacopy_v0));
}

/* Writes `value` as the redirect xattr of `path`. */
static inline void set_redirect(const char *path, const char *value)
{
	set_xattr_value(path, REDIRECT_XATTR, value, strlen(value));
}

/*
 * Marks `path` opaque with the value "y", the marker the kernel's opaque probe
 * accepts (`OverlayXattrType::is_positive_on`,
 * `kernel/core/src/fs/fs_impls/overlayfs/inode/xattr.rs`).
 */
static inline void set_opaque(const char *path)
{
	set_xattr_value(path, OPAQUE_XATTR, "y", 1);
}

/* Creates a 0:0 char device (a whiteout) at `path`. */
static inline void create_whiteout(const char *path)
{
	CHECK(mknod(path, S_IFCHR | 0600, makedev(0, 0)));
}

/* Returns 1 when `path` has no metacopy xattr, 0 otherwise. */
static inline int metacopy_absent(const char *path)
{
	return xattr_absent(path, METACOPY_XATTR);
}

/*
 * Returns 1 when `path` has a metacopy xattr, whatever value it carries, 0
 * otherwise. The record bytes belong to whichever kernel wrote them, so a
 * publication check uses this instead of an exact-byte match.
 */
static inline int metacopy_present(const char *path)
{
	char value[128];
	int found = getxattr(path, METACOPY_XATTR, value, sizeof(value)) != -1;

	errno = 0;
	return found;
}

/*
 * Prints the `len` bytes at `buf` under `tag`, so a mismatch shows what it
 * read.
 */
static inline void report_body(const char *tag, const char *buf, ssize_t len)
{
	fprintf(stderr, "    observed %s: len=%lld \"%.*s\"\n", tag,
		(long long)len, (int)(len < 0 ? 0 : len), buf);
}

#endif
