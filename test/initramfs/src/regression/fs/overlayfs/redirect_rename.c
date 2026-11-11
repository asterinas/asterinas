// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_redirect_rename"

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/xattr.h>
#include <unistd.h>

#include "ovl_common.h"
#include "ovl_mounts.h"

#define ON_BASE BASE_DIR "/on"
#define FOLLOW_BASE BASE_DIR "/follow"
#define NOFOLLOW_BASE BASE_DIR "/nofollow"

#define OVERLONG_LEVELS 6
#define OVERLONG_NAME_LEN 48

/* Returns 1 when the redirect xattr of `path` equals `expected`, 0 otherwise. */
static int redirect_matches(const char *path, const char *expected)
{
	return xattr_matches(path, REDIRECT_XATTR, expected, strlen(expected));
}

/* Returns 1 when the opaque xattr of `path` is set to "y", 0 otherwise. */
static int opaque_is_set(const char *path)
{
	char value[8];
	ssize_t written = getxattr(path, OPAQUE_XATTR, value, sizeof(value));
	int present = written == 1 && value[0] == 'y';

	errno = 0;
	return present;
}

/* Returns 1 when `path` is a 0:0 char device (a whiteout), 0 otherwise. */
static int is_whiteout(const char *path)
{
	struct stat st;
	int whiteout = lstat(path, &st) == 0 && S_ISCHR(st.st_mode) &&
		       st.st_rdev == makedev(0, 0);

	errno = 0;
	return whiteout;
}

/* Returns 1 when `path` does not exist (ENOENT), 0 otherwise. */
static int path_absent(const char *path)
{
	struct stat st;
	int absent = lstat(path, &st) == -1 && errno == ENOENT;

	errno = 0;
	return absent;
}

/* Appends "/name" to `path` in place, aborting on truncation. */
static void append_name(char *path, size_t size, const char *name)
{
	size_t used = strlen(path);
	int ret = snprintf(path + used, size - used, "/%s", name);

	CHECK_WITH(ret, _ret >= 0 && (size_t)_ret < size - used);
}

/*
 * Writes the deterministic 48-byte overlong component for `level` (1..6)
 * into `out`: "deep<level>_" followed by fill bytes. Aborts on truncation
 * or a wrong prefix length.
 */
static void format_overlong_name(char *out, size_t size, int level)
{
	int ret = snprintf(out, size, "deep%d_", level);

	CHECK_WITH(ret, _ret == 6);
	CHECK_WITH(size, _ret > OVERLONG_NAME_LEN);
	memset(out + 6, 'a' + (level - 1), OVERLONG_NAME_LEN - 6);
	out[OVERLONG_NAME_LEN] = '\0';
}

static void build_cross_parent_on_fixture(void)
{
	/* Renaming across parents records an absolute redirect to the old path. */
	create_dir(ON_BASE "/lower/cross_parent");
	create_dir(ON_BASE "/lower/cross_parent/source_parent");
	create_dir(ON_BASE "/lower/cross_parent/source_parent/origin");
	write_file(ON_BASE "/lower/cross_parent/source_parent/origin/file",
		   "cross-parent");
	create_dir(ON_BASE "/lower/cross_parent/target_parent");
}

static void build_same_parent_on_fixture(void)
{
	/* Renaming within the parent records a redirect to the old sibling. */
	create_dir(ON_BASE "/lower/same_parent");
	create_dir(ON_BASE "/lower/same_parent/parent");
	create_dir(ON_BASE "/lower/same_parent/parent/origin");
	write_file(ON_BASE "/lower/same_parent/parent/origin/file",
		   "same-parent");
}

static void build_exchange_target_on_fixture(void)
{
	/* RENAME_EXCHANGE swaps two lower dirs and records redirects both ways. */
	create_dir(ON_BASE "/lower/exchange");
	create_dir(ON_BASE "/lower/exchange/parent");
	create_dir(ON_BASE "/lower/exchange/parent/source");
	write_file(ON_BASE "/lower/exchange/parent/source/source_file",
		   "src-data");
	create_dir(ON_BASE "/lower/exchange/parent/target");
	write_file(ON_BASE "/lower/exchange/parent/target/target_file",
		   "tgt-data");
}

static void build_follow_exdev_fixture(void)
{
	/* With redirect_dir=follow a cross-parent rename of a lower dir fails EXDEV. */
	create_dir(FOLLOW_BASE "/lower/exdev");
	create_dir(FOLLOW_BASE "/lower/exdev/source_parent");
	create_dir(FOLLOW_BASE "/lower/exdev/source_parent/origin");
	write_file(FOLLOW_BASE "/lower/exdev/source_parent/origin/file",
		   "exdev-follow");
	create_dir(FOLLOW_BASE "/lower/exdev/target_parent");
}

static void build_nofollow_exdev_fixture(void)
{
	/* With redirect_dir=nofollow the same rename fails EXDEV. */
	create_dir(NOFOLLOW_BASE "/lower/exdev");
	create_dir(NOFOLLOW_BASE "/lower/exdev/source_parent");
	create_dir(NOFOLLOW_BASE "/lower/exdev/source_parent/origin");
	write_file(NOFOLLOW_BASE "/lower/exdev/source_parent/origin/file",
		   "exdev-nofollow");
	create_dir(NOFOLLOW_BASE "/lower/exdev/target_parent");
}

static void build_ancestor_chain_on_fixture(void)
{
	/* Moving a dir out of a redirected alias rewrites the redirect chain. */
	create_dir(ON_BASE "/lower/chain");
	create_dir(ON_BASE "/lower/chain/root");
	create_dir(ON_BASE "/lower/chain/root/middle");
	create_dir(ON_BASE "/lower/chain/root/middle/origin");
	write_file(ON_BASE "/lower/chain/root/middle/origin/file",
		   "ancestor-chain");
	create_dir(ON_BASE "/upper/chain");
	create_dir(ON_BASE "/upper/chain/alias");
	set_redirect(ON_BASE "/upper/chain/alias", "/chain/root");
	create_dir(ON_BASE "/upper/chain/alias/middle");
}

static void build_overlong_exdev_fixture(void)
{
	/* Six stacked 48-byte components make the rename undirectable (EXDEV). */
	char path[PATH_MAX], name[OVERLONG_NAME_LEN + 1];

	join_path(path, sizeof(path), ON_BASE, "lower");
	for (int level = 1; level <= OVERLONG_LEVELS; level++) {
		format_overlong_name(name, sizeof(name), level);
		append_name(path, sizeof(path), name);
		create_dir(path);
	}
	append_name(path, sizeof(path), "origin");
	create_dir(path);
	append_name(path, sizeof(path), "file");
	write_file(path, "overlong");
}

static void build_cross_parent_opaque_redirect_on_fixture(void)
{
	/* Renaming onto a whiteout must not mark the moved dir opaque. */
	create_dir(ON_BASE "/lower/opaque_cross");
	create_dir(ON_BASE "/lower/opaque_cross/source_parent");
	create_dir(ON_BASE "/lower/opaque_cross/source_parent/origin");
	write_file(ON_BASE "/lower/opaque_cross/source_parent/origin/file",
		   "opaque-cross");
	create_dir(ON_BASE "/lower/opaque_cross/target_parent");
	create_dir(ON_BASE "/lower/opaque_cross/target_parent/origin");
	write_file(ON_BASE
		   "/lower/opaque_cross/target_parent/origin/stray_file",
		   "unrelated-lower");
	create_dir(ON_BASE "/upper/opaque_cross");
	create_dir(ON_BASE "/upper/opaque_cross/target_parent");
	create_whiteout(ON_BASE "/upper/opaque_cross/target_parent/origin");
}

static void build_same_parent_opaque_redirect_on_fixture(void)
{
	/* Same-parent rename onto a whiteout keeps the redirect non-opaque. */
	create_dir(ON_BASE "/lower/opaque_same");
	create_dir(ON_BASE "/lower/opaque_same/parent");
	create_dir(ON_BASE "/lower/opaque_same/parent/origin");
	write_file(ON_BASE "/lower/opaque_same/parent/origin/file",
		   "opaque-same");
	create_dir(ON_BASE "/lower/opaque_same/parent/taken");
	write_file(ON_BASE "/lower/opaque_same/parent/taken/stray_file",
		   "unrelated-same");
	create_dir(ON_BASE "/upper/opaque_same");
	create_dir(ON_BASE "/upper/opaque_same/parent");
	create_whiteout(ON_BASE "/upper/opaque_same/parent/taken");
}

static void build_exchange_pure_upper_target_on_fixture(void)
{
	/* Exchanging a lower dir with a pure-upper target marks it opaque. */
	create_dir(ON_BASE "/lower/exchange_upper");
	create_dir(ON_BASE "/lower/exchange_upper/lower_parent");
	create_dir(ON_BASE "/lower/exchange_upper/lower_parent/source");
	write_file(ON_BASE "/lower/exchange_upper/lower_parent/source/file",
		   "exchange-target-opaque");
	create_dir(ON_BASE "/upper/exchange_upper");
	create_dir(ON_BASE "/upper/exchange_upper/pure_upper");
}

FN_SETUP(init)
{
	create_dir(BASE_DIR);

	create_overlay_tree_at(ON_BASE);
	create_overlay_tree_at(FOLLOW_BASE);
	create_overlay_tree_at(NOFOLLOW_BASE);

	build_cross_parent_on_fixture();
	build_same_parent_on_fixture();
	build_exchange_target_on_fixture();
	build_follow_exdev_fixture();
	build_nofollow_exdev_fixture();
	build_ancestor_chain_on_fixture();
	build_overlong_exdev_fixture();
	build_cross_parent_opaque_redirect_on_fixture();
	build_same_parent_opaque_redirect_on_fixture();
	build_exchange_pure_upper_target_on_fixture();

	mount_overlay_at(ON_BASE, ",redirect_dir=on");
	mount_overlay_at(FOLLOW_BASE, ",redirect_dir=follow");
	mount_overlay_at(NOFOLLOW_BASE, ",redirect_dir=nofollow");
}

END_SETUP()

FN_TEST(r01_cross_parent_on)
{
	struct stat st;
	char buf[64];

	TEST_SUCC(rename(ON_BASE "/merged/cross_parent/source_parent/origin",
			 ON_BASE "/merged/cross_parent/target_parent/origin"));
	read_body(ON_BASE "/merged/cross_parent/target_parent/origin/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "cross-parent"), _ret == 0);
	TEST_RES(path_absent(ON_BASE
			     "/merged/cross_parent/source_parent/origin"),
		 _ret == 1);
	TEST_RES(is_whiteout(ON_BASE
			     "/upper/cross_parent/source_parent/origin"),
		 _ret == 1);
	TEST_RES(lstat(ON_BASE "/upper/cross_parent/target_parent/origin", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(redirect_matches(ON_BASE
				  "/upper/cross_parent/target_parent/origin",
				  "/cross_parent/source_parent/origin"),
		 _ret == 1);
	TEST_RES(find_entry(ON_BASE "/merged/cross_parent/target_parent/origin",
			    "file"),
		 _ret == 1);
	TEST_RES(count_entries(ON_BASE
			       "/merged/cross_parent/target_parent/origin"),
		 _ret == 1);
}

END_TEST()

FN_TEST(r02_same_parent_on)
{
	struct stat st;
	char buf[64];

	TEST_SUCC(rename(ON_BASE "/merged/same_parent/parent/origin",
			 ON_BASE "/merged/same_parent/parent/renamed"));
	read_body(ON_BASE "/merged/same_parent/parent/renamed/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "same-parent"), _ret == 0);
	TEST_RES(path_absent(ON_BASE "/merged/same_parent/parent/origin"),
		 _ret == 1);
	TEST_RES(lstat(ON_BASE "/upper/same_parent/parent/renamed", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(redirect_matches(ON_BASE "/upper/same_parent/parent/renamed",
				  "origin"),
		 _ret == 1);
	TEST_RES(is_whiteout(ON_BASE "/upper/same_parent/parent/origin"),
		 _ret == 1);
	TEST_RES(find_entry(ON_BASE "/merged/same_parent/parent/renamed",
			    "file"),
		 _ret == 1);
	TEST_RES(count_entries(ON_BASE "/merged/same_parent/parent/renamed"),
		 _ret == 1);
}

END_TEST()

FN_TEST(r03_exchange_target_on)
{
	struct stat st;
	char buf[64];

	TEST_SUCC(renameat2(AT_FDCWD, ON_BASE "/merged/exchange/parent/source",
			    AT_FDCWD, ON_BASE "/merged/exchange/parent/target",
			    RENAME_EXCHANGE));
	read_body(ON_BASE "/merged/exchange/parent/source/target_file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "tgt-data"), _ret == 0);
	read_body(ON_BASE "/merged/exchange/parent/target/source_file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "src-data"), _ret == 0);
	TEST_RES(find_entry(ON_BASE "/merged/exchange/parent/source",
			    "target_file"),
		 _ret == 1);
	TEST_RES(count_entries(ON_BASE "/merged/exchange/parent/source"),
		 _ret == 1);
	TEST_RES(find_entry(ON_BASE "/merged/exchange/parent/target",
			    "source_file"),
		 _ret == 1);
	TEST_RES(count_entries(ON_BASE "/merged/exchange/parent/target"),
		 _ret == 1);
	TEST_RES(lstat(ON_BASE "/upper/exchange/parent/source", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(redirect_matches(ON_BASE "/upper/exchange/parent/source",
				  "target"),
		 _ret == 1);
	TEST_RES(lstat(ON_BASE "/upper/exchange/parent/target", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(redirect_matches(ON_BASE "/upper/exchange/parent/target",
				  "source"),
		 _ret == 1);
}

END_TEST()

FN_TEST(r04_follow_exdev)
{
	struct stat st;
	char buf[64];

	TEST_SUCC(lstat(FOLLOW_BASE "/merged/exdev/source_parent/origin", &st));
	TEST_ERRNO(rename(FOLLOW_BASE "/merged/exdev/source_parent/origin",
			  FOLLOW_BASE "/merged/exdev/target_parent/origin"),
		   EXDEV);
	read_body(FOLLOW_BASE "/merged/exdev/source_parent/origin/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "exdev-follow"), _ret == 0);
	TEST_RES(path_absent(FOLLOW_BASE "/upper/exdev/source_parent"),
		 _ret == 1);
	TEST_RES(path_absent(FOLLOW_BASE "/upper/exdev/target_parent"),
		 _ret == 1);
	TEST_RES(count_entries(FOLLOW_BASE "/upper"), _ret == 0);
}

END_TEST()

FN_TEST(r05_nofollow_exdev)
{
	struct stat st;
	char buf[64];

	TEST_SUCC(
		lstat(NOFOLLOW_BASE "/merged/exdev/source_parent/origin", &st));
	TEST_ERRNO(rename(NOFOLLOW_BASE "/merged/exdev/source_parent/origin",
			  NOFOLLOW_BASE "/merged/exdev/target_parent/origin"),
		   EXDEV);
	read_body(NOFOLLOW_BASE "/merged/exdev/source_parent/origin/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "exdev-nofollow"), _ret == 0);
	TEST_RES(path_absent(NOFOLLOW_BASE "/upper/exdev/source_parent"),
		 _ret == 1);
	TEST_RES(path_absent(NOFOLLOW_BASE "/upper/exdev/target_parent"),
		 _ret == 1);
	TEST_RES(count_entries(NOFOLLOW_BASE "/upper"), _ret == 0);
}

END_TEST()

FN_TEST(r06_ancestor_chain_on)
{
	struct stat st;
	char buf[64];

	TEST_SUCC(rename(ON_BASE "/merged/chain/alias/middle/origin",
			 ON_BASE "/merged/chain/relocated"));
	TEST_RES(lstat(ON_BASE "/upper/chain/relocated", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(redirect_matches(ON_BASE "/upper/chain/relocated",
				  "/chain/root/middle/origin"),
		 _ret == 1);
	read_body(ON_BASE "/merged/chain/relocated/file", buf, sizeof(buf));
	TEST_RES(strcmp(buf, "ancestor-chain"), _ret == 0);
	TEST_RES(is_whiteout(ON_BASE "/upper/chain/alias/middle/origin"),
		 _ret == 1);
	TEST_RES(path_absent(ON_BASE "/merged/chain/alias/middle/origin"),
		 _ret == 1);
}

END_TEST()

FN_TEST(r07_overlong_exdev)
{
	char src[PATH_MAX], dst[PATH_MAX], file[PATH_MAX];
	char upper_first[PATH_MAX], name[OVERLONG_NAME_LEN + 1];
	char buf[64];

	join_path(src, sizeof(src), ON_BASE, "merged");
	for (int level = 1; level <= OVERLONG_LEVELS; level++) {
		format_overlong_name(name, sizeof(name), level);
		append_name(src, sizeof(src), name);
	}
	append_name(src, sizeof(src), "origin");
	join_path(dst, sizeof(dst), ON_BASE "/merged", "short_target");

	TEST_ERRNO(rename(src, dst), EXDEV);

	join_path(file, sizeof(file), src, "file");
	read_body(file, buf, sizeof(buf));
	TEST_RES(strcmp(buf, "overlong"), _ret == 0);

	format_overlong_name(name, sizeof(name), 1);
	join_path(upper_first, sizeof(upper_first), ON_BASE, "upper");
	append_name(upper_first, sizeof(upper_first), name);
	TEST_RES(path_absent(upper_first), _ret == 1);
}

END_TEST()

FN_TEST(r08_cross_parent_opaque_redirect_on)
{
	struct stat st;
	char buf[64];

	TEST_SUCC(rename(ON_BASE "/merged/opaque_cross/source_parent/origin",
			 ON_BASE "/merged/opaque_cross/target_parent/origin"));
	read_body(ON_BASE "/merged/opaque_cross/target_parent/origin/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "opaque-cross"), _ret == 0);
	TEST_RES(
		path_absent(
			ON_BASE
			"/merged/opaque_cross/target_parent/origin/stray_file"),
		_ret == 1);
	TEST_RES(find_entry(ON_BASE "/merged/opaque_cross/target_parent/origin",
			    "file"),
		 _ret == 1);
	TEST_RES(count_entries(ON_BASE
			       "/merged/opaque_cross/target_parent/origin"),
		 _ret == 1);
	TEST_RES(lstat(ON_BASE "/upper/opaque_cross/target_parent/origin", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(redirect_matches(ON_BASE
				  "/upper/opaque_cross/target_parent/origin",
				  "/opaque_cross/source_parent/origin"),
		 _ret == 1);
	TEST_RES(xattr_absent(ON_BASE
			      "/upper/opaque_cross/target_parent/origin",
			      OPAQUE_XATTR),
		 _ret == 1);
	TEST_RES(is_whiteout(ON_BASE
			     "/upper/opaque_cross/source_parent/origin"),
		 _ret == 1);
	TEST_RES(path_absent(ON_BASE
			     "/merged/opaque_cross/source_parent/origin"),
		 _ret == 1);
}

END_TEST()

FN_TEST(r09_same_parent_opaque_redirect_on)
{
	struct stat st;
	char buf[64];

	TEST_SUCC(rename(ON_BASE "/merged/opaque_same/parent/origin",
			 ON_BASE "/merged/opaque_same/parent/taken"));
	read_body(ON_BASE "/merged/opaque_same/parent/taken/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "opaque-same"), _ret == 0);
	TEST_RES(path_absent(ON_BASE
			     "/merged/opaque_same/parent/taken/stray_file"),
		 _ret == 1);
	TEST_RES(find_entry(ON_BASE "/merged/opaque_same/parent/taken", "file"),
		 _ret == 1);
	TEST_RES(count_entries(ON_BASE "/merged/opaque_same/parent/taken"),
		 _ret == 1);
	TEST_RES(lstat(ON_BASE "/upper/opaque_same/parent/taken", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(redirect_matches(ON_BASE "/upper/opaque_same/parent/taken",
				  "origin"),
		 _ret == 1);
	TEST_RES(xattr_absent(ON_BASE "/upper/opaque_same/parent/taken",
			      OPAQUE_XATTR),
		 _ret == 1);
	TEST_RES(is_whiteout(ON_BASE "/upper/opaque_same/parent/origin"),
		 _ret == 1);
	TEST_RES(path_absent(ON_BASE "/merged/opaque_same/parent/origin"),
		 _ret == 1);
}

END_TEST()

FN_TEST(r10_exchange_pure_upper_target_on)
{
	struct stat st;
	char buf[64];

	TEST_SUCC(renameat2(
		AT_FDCWD, ON_BASE "/merged/exchange_upper/lower_parent/source",
		AT_FDCWD, ON_BASE "/merged/exchange_upper/pure_upper",
		RENAME_EXCHANGE));
	TEST_RES(lstat(ON_BASE "/merged/exchange_upper/lower_parent/source",
		       &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(count_entries(ON_BASE
			       "/merged/exchange_upper/lower_parent/source"),
		 _ret == 0);
	TEST_RES(path_absent(ON_BASE
			     "/merged/exchange_upper/lower_parent/source/file"),
		 _ret == 1);
	read_body(ON_BASE "/merged/exchange_upper/pure_upper/file", buf,
		  sizeof(buf));
	TEST_RES(strcmp(buf, "exchange-target-opaque"), _ret == 0);
	TEST_RES(find_entry(ON_BASE "/merged/exchange_upper/pure_upper",
			    "file"),
		 _ret == 1);
	TEST_RES(count_entries(ON_BASE "/merged/exchange_upper/pure_upper"),
		 _ret == 1);
	TEST_RES(lstat(ON_BASE "/upper/exchange_upper/pure_upper", &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(redirect_matches(ON_BASE "/upper/exchange_upper/pure_upper",
				  "/exchange_upper/lower_parent/source"),
		 _ret == 1);
	TEST_RES(lstat(ON_BASE "/upper/exchange_upper/lower_parent/source",
		       &st),
		 _ret == 0 && S_ISDIR(st.st_mode));
	TEST_RES(opaque_is_set(ON_BASE
			       "/upper/exchange_upper/lower_parent/source"),
		 _ret == 1);
	TEST_RES(xattr_absent(ON_BASE
			      "/upper/exchange_upper/lower_parent/source",
			      REDIRECT_XATTR),
		 _ret == 1);
	TEST_RES(xattr_absent(ON_BASE "/upper/exchange_upper/pure_upper",
			      OPAQUE_XATTR),
		 _ret == 1);
}

END_TEST()

FN_SETUP(cleanup)
{
	unmount_overlay_at(NOFOLLOW_BASE);
	unmount_overlay_at(FOLLOW_BASE);
	unmount_overlay_at(ON_BASE);
	CHECK(remove_tree(BASE_DIR));
}

END_SETUP()
