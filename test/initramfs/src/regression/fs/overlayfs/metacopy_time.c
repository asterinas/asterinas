// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_metacopy_time"

#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#include "ovl_common.h"
#include "ovl_mounts.h"

#define ROOT BASE_DIR "/data"

#define MOUNT_OPTIONS ",metacopy=on,redirect_dir=on"

#define LOWER_DATA "lower-data"
#define LOWER_DATA_LEN (sizeof(LOWER_DATA) - 1)

/*
 * Both fixture timestamps are 2000-01-01, far enough from the mount clock that
 * a timestamp frozen by copy-up cannot be mistaken for "now".
 */
#define OLD_TIMESTAMP_SEC 946684800

/*
 * One lower entry per case, so no case can change the file state another case
 * observes. Every case gets its metadata-only copy-up from the mount itself
 * (`chmod` on the merged name), never from a fabricated record.
 */
static const char *const case_names[] = { "mt01", "mt02", "mt03" };

/* Writes the path of `part` ("lower", "upper" or "merged") of case `name`. */
static void case_path(char *out, size_t size, const char *part,
		      const char *name)
{
	char dir[PATH_MAX];

	join_path(dir, sizeof(dir), ROOT, part);
	join_path(out, size, dir, name);
}

/* Sets the fixture's old atime and mtime on the lower file at `path`. */
static void set_old_timestamps(const char *path)
{
	const struct timespec times[2] = { { OLD_TIMESTAMP_SEC, 0 },
					   { OLD_TIMESTAMP_SEC, 0 } };

	CHECK(utimensat(AT_FDCWD, path, times, 0));
}

/* Returns 1 when `st` carries the fixture's old mtime, 0 otherwise. */
static int mtime_is_old(const struct stat *st)
{
	return st->st_mtim.tv_sec == OLD_TIMESTAMP_SEC &&
	       st->st_mtim.tv_nsec == 0;
}

/*
 * Prints the raw mtime `st` carries, so a failed timestamp assertion reports
 * the value it observed and not only the boolean it wanted.
 */
static void report_mtime(const char *tag, const struct stat *st)
{
	fprintf(stderr, "    observed %s: st_mtim=%lld.%09lld\n", tag,
		(long long)st->st_mtim.tv_sec, (long long)st->st_mtim.tv_nsec);
}

/*
 * Creates the lower file of each case and gives it the fixture's old
 * timestamps.
 */
static void build_lower_files(void)
{
	char path[PATH_MAX];
	struct stat st;
	size_t i;

	for (i = 0; i < sizeof(case_names) / sizeof(case_names[0]); i++) {
		case_path(path, sizeof(path), "lower", case_names[i]);
		write_file(path, LOWER_DATA);
		set_old_timestamps(path);
		/* A case whose lower timestamp is not old would test nothing. */
		CHECK(stat(path, &st));
		CHECK_WITH(mtime_is_old(&st), _ret == 1);
	}
}

FN_SETUP(init)
{
	create_dir(BASE_DIR);
	create_overlay_tree_at(ROOT);
	build_lower_files();
	mount_overlay_at(ROOT, MOUNT_OPTIONS);
}

END_SETUP()

FN_TEST(mt01_metadata_copy_up_preserves_mtime)
{
	char merged[PATH_MAX], upper[PATH_MAX];
	struct stat st;

	case_path(merged, sizeof(merged), "merged", "mt01");
	case_path(upper, sizeof(upper), "upper", "mt01");

	/* `chmod` is metadata only: it must copy the file up without its data. */
	TEST_SUCC(chmod(merged, 0640));

	TEST_RES(stat(merged, &st), _ret == 0);
	report_mtime("merged after metadata copy-up", &st);
	TEST_RES(mtime_is_old(&st), _ret == 1);

	TEST_RES(stat(upper, &st), _ret == 0);
	report_mtime("upper after metadata copy-up", &st);
	TEST_RES(mtime_is_old(&st), _ret == 1);

	/* The copy-up really was metadata only, so data still reads from lower. */
	TEST_RES(metacopy_present(upper), _ret == 1);
}

END_TEST()

FN_TEST(mt02_read_only_open_keeps_mtime)
{
	char merged[PATH_MAX], upper[PATH_MAX];
	struct stat st;
	int fd;

	case_path(merged, sizeof(merged), "merged", "mt02");
	case_path(upper, sizeof(upper), "upper", "mt02");

	TEST_SUCC(chmod(merged, 0640));
	/* Without the record the open below would have no deferred copy to skip. */
	TEST_RES(metacopy_present(upper), _ret == 1);

	fd = TEST_SUCC(open(merged, O_RDONLY));
	TEST_SUCC(close(fd));

	TEST_RES(stat(merged, &st), _ret == 0);
	report_mtime("merged after read-only open", &st);
	TEST_RES(mtime_is_old(&st), _ret == 1);

	/* A read-only open must not complete the deferred data copy either. */
	TEST_RES(metacopy_present(upper), _ret == 1);
}

END_TEST()

/*
 * The writable open completes the deferred data copy, and that copy is a write
 * of the whole lower body into upper. Linux's ovl_copy_up_meta_inode_data()
 * (`fs/overlayfs/copy_up.c:1082-1129`) performs that copy and removes the
 * metacopy record, while the timestamp restore that
 * `ovl_copy_up_metadata()` (`copy_up.c:717`) performs for the initial copy-up
 * is not repeated here. So this case expects `mtime` to advance even though the
 * caller writes nothing: it encodes the Linux source's behavior, not a POSIX
 * expectation that merely opening a file leaves it untouched.
 */
FN_TEST(mt03_writable_open_advances_mtime)
{
	char merged[PATH_MAX], upper[PATH_MAX], buf[64];
	struct stat st;
	int fd;

	case_path(merged, sizeof(merged), "merged", "mt03");
	case_path(upper, sizeof(upper), "upper", "mt03");

	TEST_SUCC(chmod(merged, 0640));
	TEST_RES(metacopy_present(upper), _ret == 1);

	fd = TEST_SUCC(open(merged, O_RDWR));
	TEST_SUCC(close(fd));

	TEST_RES(stat(merged, &st), _ret == 0);
	report_mtime("merged after writable open", &st);
	TEST_RES(mtime_is_old(&st), _ret == 0);

	/* The record falling away and the data arriving are one event. */
	TEST_RES(metacopy_absent(upper), _ret == 1);
	TEST_RES(read_body(upper, buf, sizeof(buf)), _ret == LOWER_DATA_LEN);
	TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);
}

END_TEST()

FN_SETUP(cleanup)
{
	unmount_overlay_at(ROOT);
	CHECK(remove_tree(BASE_DIR));
}

END_SETUP()
