// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_redirect_follow"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <string.h>
#include <sys/stat.h>

#include "ovl_common.h"
#include "ovl_mounts.h"

#define FOLLOW_BASE BASE_DIR "/follow"
#define ON_BASE BASE_DIR "/on"
#define NOFOLLOW_BASE BASE_DIR "/nofollow"
#define OFF_BASE BASE_DIR "/off"
#define MULTI_BASE BASE_DIR "/multi"

static void build_relative_follow_fixture(void)
{
	/* An upper dir renamed away and redirected back to a lower sibling. */
	create_dir(FOLLOW_BASE "/lower/relative");
	create_dir(FOLLOW_BASE "/lower/relative/parent");
	create_dir(FOLLOW_BASE "/lower/relative/parent/origin");
	write_file(FOLLOW_BASE "/lower/relative/parent/origin/file",
		   "relative-lower");
	create_dir(FOLLOW_BASE "/upper/relative");
	create_dir(FOLLOW_BASE "/upper/relative/parent");
	create_dir(FOLLOW_BASE "/upper/relative/parent/renamed");
	set_redirect(FOLLOW_BASE "/upper/relative/parent/renamed", "origin");
}

static void build_absolute_follow_fixture(void)
{
	/* An absolute redirect resolved from the layer root, not the parent. */
	create_dir(FOLLOW_BASE "/lower/absolute");
	create_dir(FOLLOW_BASE "/lower/absolute/elsewhere");
	write_file(FOLLOW_BASE "/lower/absolute/elsewhere/file",
		   "absolute-lower");
	create_dir(FOLLOW_BASE "/upper/absolute");
	create_dir(FOLLOW_BASE "/upper/absolute/holder");
	create_dir(FOLLOW_BASE "/upper/absolute/holder/renamed");
	set_redirect(FOLLOW_BASE "/upper/absolute/holder/renamed",
		     "/absolute/elsewhere");
}

static void build_absolute_lower_rebase_fixture(void)
{
	/* An absolute redirect rebased onto the bottom of two lower layers. */
	create_dir(MULTI_BASE "/top");
	create_dir(MULTI_BASE "/bottom");
	create_dir(MULTI_BASE "/top/rebased");
	create_dir(MULTI_BASE "/top/rebased/holder");
	create_dir(MULTI_BASE "/top/rebased/holder/renamed");
	set_redirect(MULTI_BASE "/top/rebased/holder/renamed",
		     "/rebased/elsewhere");
	create_dir(MULTI_BASE "/bottom/rebased");
	create_dir(MULTI_BASE "/bottom/rebased/elsewhere");
	write_file(MULTI_BASE "/bottom/rebased/elsewhere/file", "multi-lower");
}

static void build_relative_on_fixture(void)
{
	/* Redirect_dir=on follows a relative redirect just like follow mode. */
	create_dir(ON_BASE "/lower/relative");
	create_dir(ON_BASE "/lower/relative/parent");
	create_dir(ON_BASE "/lower/relative/parent/origin");
	write_file(ON_BASE "/lower/relative/parent/origin/file", "relative-on");
	create_dir(ON_BASE "/upper/relative");
	create_dir(ON_BASE "/upper/relative/parent");
	create_dir(ON_BASE "/upper/relative/parent/renamed");
	set_redirect(ON_BASE "/upper/relative/parent/renamed", "origin");
}

static void build_nofollow_negative_fixture(void)
{
	/* Redirect_dir=nofollow keeps the record but hides the target. */
	create_dir(NOFOLLOW_BASE "/lower/relative");
	create_dir(NOFOLLOW_BASE "/lower/relative/parent");
	create_dir(NOFOLLOW_BASE "/lower/relative/parent/origin");
	write_file(NOFOLLOW_BASE "/lower/relative/parent/origin/file",
		   "hidden-nofollow");
	create_dir(NOFOLLOW_BASE "/upper/relative");
	create_dir(NOFOLLOW_BASE "/upper/relative/parent");
	create_dir(NOFOLLOW_BASE "/upper/relative/parent/renamed");
	set_redirect(NOFOLLOW_BASE "/upper/relative/parent/renamed", "origin");
}

static void build_off_negative_fixture(void)
{
	/* Redirect_dir=off keeps the record but hides the target. */
	create_dir(OFF_BASE "/lower/relative");
	create_dir(OFF_BASE "/lower/relative/parent");
	create_dir(OFF_BASE "/lower/relative/parent/origin");
	write_file(OFF_BASE "/lower/relative/parent/origin/file", "hidden-off");
	create_dir(OFF_BASE "/upper/relative");
	create_dir(OFF_BASE "/upper/relative/parent");
	create_dir(OFF_BASE "/upper/relative/parent/renamed");
	set_redirect(OFF_BASE "/upper/relative/parent/renamed", "origin");
}

static void build_invalid_double_slash_fixture(void)
{
	/* A redirect value with two leading slashes is rejected with EINVAL. */
	create_dir(FOLLOW_BASE "/upper/double_slash");
	set_redirect(FOLLOW_BASE "/upper/double_slash", "//target");
}

static void build_invalid_trailing_slash_fixture(void)
{
	/* A redirect value with a trailing slash is rejected with EINVAL. */
	create_dir(FOLLOW_BASE "/upper/trailing_slash");
	set_redirect(FOLLOW_BASE "/upper/trailing_slash", "/target/");
}

static void build_invalid_relative_slash_fixture(void)
{
	/* A relative redirect value containing a slash is rejected with EINVAL. */
	create_dir(FOLLOW_BASE "/upper/relative_slash");
	set_redirect(FOLLOW_BASE "/upper/relative_slash", "first/second");
}

static void build_zero_length_fixture(void)
{
	/* A zero-length redirect value is preserved but never followed. */
	create_dir(FOLLOW_BASE "/upper/zero_length");
	set_xattr_value(FOLLOW_BASE "/upper/zero_length", REDIRECT_XATTR, "",
			0);
}

FN_SETUP(init)
{
	create_dir(BASE_DIR);

	create_overlay_tree_at(FOLLOW_BASE);
	create_overlay_tree_at(ON_BASE);
	create_overlay_tree_at(MULTI_BASE);
	create_overlay_tree_at(NOFOLLOW_BASE);
	create_overlay_tree_at(OFF_BASE);

	build_relative_follow_fixture();
	build_absolute_follow_fixture();
	build_absolute_lower_rebase_fixture();
	build_relative_on_fixture();
	build_nofollow_negative_fixture();
	build_off_negative_fixture();
	build_invalid_double_slash_fixture();
	build_invalid_trailing_slash_fixture();
	build_invalid_relative_slash_fixture();
	build_zero_length_fixture();

	mount_overlay_at(FOLLOW_BASE, ",redirect_dir=follow");
	mount_overlay_at(ON_BASE, ",redirect_dir=on");
	mount_overlay_at(NOFOLLOW_BASE, ",redirect_dir=nofollow");
	mount_overlay_at(OFF_BASE, ",redirect_dir=off");
	mount_overlay_multi_lower_at(MULTI_BASE, ",redirect_dir=follow");
}

END_SETUP()

FN_TEST(f01_relative_follow)
{
	struct stat st;
	char buf[64];

	TEST_RES(lstat(FOLLOW_BASE "/merged/relative/parent/renamed", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	read_body(FOLLOW_BASE "/merged/relative/parent/renamed/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "relative-lower"), _ret == 0);
	TEST_RES(find_entry(FOLLOW_BASE "/merged/relative/parent/renamed",
			    "file"),
		 _ret == 1);
	TEST_RES(count_entries(FOLLOW_BASE "/merged/relative/parent/renamed"),
		 _ret == 1);
	TEST_RES(xattr_matches(FOLLOW_BASE "/upper/relative/parent/renamed",
			       REDIRECT_XATTR, "origin", strlen("origin")),
		 _ret == 1);
}

END_TEST()

FN_TEST(f02_absolute_follow)
{
	struct stat st;
	char buf[64];

	TEST_RES(lstat(FOLLOW_BASE "/merged/absolute/holder/renamed", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	read_body(FOLLOW_BASE "/merged/absolute/holder/renamed/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "absolute-lower"), _ret == 0);
	TEST_RES(find_entry(FOLLOW_BASE "/merged/absolute/holder/renamed",
			    "file"),
		 _ret == 1);
	TEST_RES(count_entries(FOLLOW_BASE "/merged/absolute/holder/renamed"),
		 _ret == 1);
	TEST_RES(xattr_matches(FOLLOW_BASE "/upper/absolute/holder/renamed",
			       REDIRECT_XATTR, "/absolute/elsewhere",
			       strlen("/absolute/elsewhere")),
		 _ret == 1);
}

END_TEST()

FN_TEST(f03_absolute_lower_rebase)
{
	struct stat st;
	char buf[64];

	TEST_RES(lstat(MULTI_BASE "/merged/rebased/holder/renamed", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	read_body(MULTI_BASE "/merged/rebased/holder/renamed/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "multi-lower"), _ret == 0);
	TEST_RES(find_entry(MULTI_BASE "/merged/rebased/holder/renamed",
			    "file"),
		 _ret == 1);
	TEST_RES(count_entries(MULTI_BASE "/merged/rebased/holder/renamed"),
		 _ret == 1);
	TEST_RES(xattr_matches(MULTI_BASE "/top/rebased/holder/renamed",
			       REDIRECT_XATTR, "/rebased/elsewhere",
			       strlen("/rebased/elsewhere")),
		 _ret == 1);
}

END_TEST()

FN_TEST(f04_relative_on)
{
	struct stat st;
	char buf[64];

	TEST_RES(lstat(ON_BASE "/merged/relative/parent/renamed", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	read_body(ON_BASE "/merged/relative/parent/renamed/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "relative-on"), _ret == 0);
	TEST_RES(find_entry(ON_BASE "/merged/relative/parent/renamed", "file"),
		 _ret == 1);
	TEST_RES(count_entries(ON_BASE "/merged/relative/parent/renamed"),
		 _ret == 1);
	TEST_RES(xattr_matches(ON_BASE "/upper/relative/parent/renamed",
			       REDIRECT_XATTR, "origin", strlen("origin")),
		 _ret == 1);
}

END_TEST()

FN_TEST(f05_nofollow_negative)
{
	struct stat st;
	char path[PATH_MAX];

	TEST_RES(lstat(NOFOLLOW_BASE "/merged/relative/parent/renamed", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	join_path(path, sizeof(path),
		  NOFOLLOW_BASE "/merged/relative/parent/renamed", "file");
	TEST_ERRNO(lstat(path, &st), ENOENT);
	TEST_ERRNO(open(path, O_RDONLY), ENOENT);
	TEST_RES(count_entries(NOFOLLOW_BASE "/merged/relative/parent/renamed"),
		 _ret == 0);
	TEST_RES(xattr_matches(NOFOLLOW_BASE "/upper/relative/parent/renamed",
			       REDIRECT_XATTR, "origin", strlen("origin")),
		 _ret == 1);
}

END_TEST()

FN_TEST(f06_off_negative)
{
	struct stat st;
	char path[PATH_MAX];

	TEST_RES(lstat(OFF_BASE "/merged/relative/parent/renamed", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	join_path(path, sizeof(path),
		  OFF_BASE "/merged/relative/parent/renamed", "file");
	TEST_ERRNO(lstat(path, &st), ENOENT);
	TEST_ERRNO(open(path, O_RDONLY), ENOENT);
	TEST_RES(count_entries(OFF_BASE "/merged/relative/parent/renamed"),
		 _ret == 0);
	TEST_RES(xattr_matches(OFF_BASE "/upper/relative/parent/renamed",
			       REDIRECT_XATTR, "origin", strlen("origin")),
		 _ret == 1);
}

END_TEST()

FN_TEST(f07_invalid_double_slash)
{
	struct stat st;

	TEST_ERRNO(lstat(FOLLOW_BASE "/merged/double_slash", &st), EINVAL);
}

END_TEST()

FN_TEST(f08_invalid_trailing_slash)
{
	struct stat st;

	TEST_ERRNO(lstat(FOLLOW_BASE "/merged/trailing_slash", &st), EINVAL);
}

END_TEST()

FN_TEST(f09_invalid_relative_slash)
{
	struct stat st;

	TEST_ERRNO(lstat(FOLLOW_BASE "/merged/relative_slash", &st), EINVAL);
}

END_TEST()

FN_TEST(f10_zero_length)
{
	struct stat st;

	TEST_RES(lstat(FOLLOW_BASE "/merged/zero_length", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(count_entries(FOLLOW_BASE "/merged/zero_length"), _ret == 0);
	TEST_RES(xattr_matches(FOLLOW_BASE "/upper/zero_length", REDIRECT_XATTR,
			       "", 0),
		 _ret == 1);
}

END_TEST()

FN_SETUP(cleanup)
{
	unmount_overlay_at(MULTI_BASE);
	unmount_overlay_at(OFF_BASE);
	unmount_overlay_at(NOFOLLOW_BASE);
	unmount_overlay_at(ON_BASE);
	unmount_overlay_at(FOLLOW_BASE);
	CHECK(remove_tree(BASE_DIR));
}

END_SETUP()
