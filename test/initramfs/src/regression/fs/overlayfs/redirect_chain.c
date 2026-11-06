// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_redirect_chain"

#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "ovl_common.h"
#include "ovl_mounts.h"

#define ROOT BASE_DIR "/chain"

/* rc03 and rc04 fabricate metacopy records, which a mount must follow. */
#define MOUNT_OPTIONS ",metacopy=on,redirect_dir=on"

#define DEEP_DATA "deep-data"
#define DEEP_DATA_LEN (sizeof(DEEP_DATA) - 1)
#define LOWER_DATA "lower-data"
#define LOWER_DATA_LEN (sizeof(LOWER_DATA) - 1)

static void build_chained_absolute_dir_redirect_fixture(void)
{
	/* Upper /alias -> /a, top /a -> /b, and only bottom /b holds the file. */
	create_dir(ROOT "/upper/rc01");
	create_dir(ROOT "/upper/rc01/alias");
	set_redirect(ROOT "/upper/rc01/alias", "/rc01/a");
	create_dir(ROOT "/top/rc01");
	create_dir(ROOT "/top/rc01/a");
	set_redirect(ROOT "/top/rc01/a", "/rc01/b");
	create_dir(ROOT "/bottom/rc01");
	create_dir(ROOT "/bottom/rc01/b");
	write_file(ROOT "/bottom/rc01/b/file", DEEP_DATA);
}

static void build_single_hop_absolute_dir_fixture(void)
{
	/* Upper /alias -> /b; a single absolute hop must keep working. */
	create_dir(ROOT "/upper/rc02");
	create_dir(ROOT "/upper/rc02/alias");
	set_redirect(ROOT "/upper/rc02/alias", "/rc02/b");
	create_dir(ROOT "/bottom/rc02");
	create_dir(ROOT "/bottom/rc02/b");
	write_file(ROOT "/bottom/rc02/b/file", DEEP_DATA);
}

static void build_metacopy_data_through_chain_fixture(void)
{
	/* The data redirect names /a/file, where /a itself redirects to /b. */
	create_dir(ROOT "/upper/rc03");
	write_zero_file(ROOT "/upper/rc03/mirror", LOWER_DATA_LEN);
	set_metacopy(ROOT "/upper/rc03/mirror");
	set_redirect(ROOT "/upper/rc03/mirror", "/rc03/a/file");
	create_dir(ROOT "/top/rc03");
	create_dir(ROOT "/top/rc03/a");
	set_redirect(ROOT "/top/rc03/a", "/rc03/b");
	create_dir(ROOT "/bottom/rc03");
	create_dir(ROOT "/bottom/rc03/b");
	write_file(ROOT "/bottom/rc03/b/file", LOWER_DATA);
}

static void build_metacopy_data_single_hop_fixture(void)
{
	/* The data redirect names the real file in one absolute hop. */
	create_dir(ROOT "/upper/rc04");
	write_zero_file(ROOT "/upper/rc04/mirror", LOWER_DATA_LEN);
	set_metacopy(ROOT "/upper/rc04/mirror");
	set_redirect(ROOT "/upper/rc04/mirror", "/rc04/b/file");
	create_dir(ROOT "/bottom/rc04");
	create_dir(ROOT "/bottom/rc04/b");
	write_file(ROOT "/bottom/rc04/b/file", LOWER_DATA);
}

static void build_absolute_then_relative_chain_fixture(void)
{
	/* The absolute walk meets a relative redirect inside the same layer. */
	create_dir(ROOT "/upper/rc05");
	create_dir(ROOT "/upper/rc05/alias");
	set_redirect(ROOT "/upper/rc05/alias", "/rc05/a");
	create_dir(ROOT "/top/rc05");
	create_dir(ROOT "/top/rc05/a");
	set_redirect(ROOT "/top/rc05/a", "b");
	create_dir(ROOT "/top/rc05/b");
	write_file(ROOT "/top/rc05/b/file", DEEP_DATA);
}

static void build_relative_redirect_continuation_fixture(void)
{
	/* A relative redirect on a lower directory must keep merging downward. */
	create_dir(ROOT "/upper/rc06");
	create_dir(ROOT "/upper/rc06/alias");
	create_dir(ROOT "/top/rc06");
	create_dir(ROOT "/top/rc06/alias");
	set_redirect(ROOT "/top/rc06/alias", "b");
	create_dir(ROOT "/bottom/rc06");
	create_dir(ROOT "/bottom/rc06/b");
	write_file(ROOT "/bottom/rc06/b/file", DEEP_DATA);
}

FN_SETUP(init)
{
	create_dir(BASE_DIR);

	create_multi_lower_tree_at(ROOT);

	build_chained_absolute_dir_redirect_fixture();
	build_single_hop_absolute_dir_fixture();
	build_metacopy_data_through_chain_fixture();
	build_metacopy_data_single_hop_fixture();
	build_absolute_then_relative_chain_fixture();
	build_relative_redirect_continuation_fixture();

	mount_overlay_multi_lower_at(ROOT, MOUNT_OPTIONS);
}

END_SETUP()

/*
 * Checks a directory-redirect case: rc01, rc02, rc05 and rc06 all assert the
 * same four facts over `/merged/<case>/alias/file`. A macro, not a helper
 * function, because the `TEST_*` macros update the counters that `FN_TEST`
 * declares as locals of the test function.
 */
#define CHECK_DIR_REDIRECT_CASE(case_name)                                    \
	do {                                                                  \
		char merged[PATH_MAX], alias[PATH_MAX], file[PATH_MAX];       \
		struct stat st;                                               \
		char buf[64];                                                 \
                                                                              \
		join_path(merged, sizeof(merged), ROOT "/merged", case_name); \
		join_path(alias, sizeof(alias), merged, "alias");             \
		join_path(file, sizeof(file), alias, "file");                 \
		TEST_RES(read_body(file, buf, sizeof(buf)),                   \
			 _ret == DEEP_DATA_LEN);                              \
		TEST_RES(strcmp(buf, DEEP_DATA), _ret == 0);                  \
		TEST_RES(lstat(file, &st),                                    \
			 _ret == 0 && st.st_size == DEEP_DATA_LEN);           \
		TEST_RES(find_entry(alias, "file"), _ret == 1);               \
	} while (0)

/*
 * Checks a metacopy-data case: rc03 and rc04 both read the data through the
 * merged name and, after a writable open, read it back in the upper layer.
 */
#define CHECK_METACOPY_DATA_CASE(case_name)                                    \
	do {                                                                   \
		char merged[PATH_MAX], upper[PATH_MAX];                        \
		char merged_file[PATH_MAX], upper_file[PATH_MAX];              \
		char buf[64];                                                  \
		int fd;                                                        \
                                                                               \
		join_path(merged, sizeof(merged), ROOT "/merged", case_name);  \
		join_path(upper, sizeof(upper), ROOT "/upper", case_name);     \
		join_path(merged_file, sizeof(merged_file), merged, "mirror"); \
		join_path(upper_file, sizeof(upper_file), upper, "mirror");    \
                                                                               \
		TEST_RES(read_body(merged_file, buf, sizeof(buf)),             \
			 _ret == LOWER_DATA_LEN);                              \
		TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);                  \
                                                                               \
		fd = TEST_SUCC(open(merged_file, O_RDWR));                     \
		TEST_SUCC(close(fd));                                          \
		TEST_RES(read_body(upper_file, buf, sizeof(buf)),              \
			 _ret == LOWER_DATA_LEN);                              \
		report_body("upper body", buf, (ssize_t)strlen(buf));          \
		TEST_RES(strcmp(buf, LOWER_DATA), _ret == 0);                  \
	} while (0)

FN_TEST(rc01_chained_absolute_dir_redirect)
{
	CHECK_DIR_REDIRECT_CASE("rc01");
}

END_TEST()

FN_TEST(rc02_single_hop_absolute_dir)
{
	CHECK_DIR_REDIRECT_CASE("rc02");
}

END_TEST()

FN_TEST(rc03_metacopy_data_through_chain)
{
	CHECK_METACOPY_DATA_CASE("rc03");
}

END_TEST()

FN_TEST(rc04_metacopy_data_single_hop)
{
	CHECK_METACOPY_DATA_CASE("rc04");
}

END_TEST()

FN_TEST(rc05_absolute_then_relative_chain)
{
	CHECK_DIR_REDIRECT_CASE("rc05");
}

END_TEST()

FN_TEST(rc06_relative_redirect_continuation)
{
	CHECK_DIR_REDIRECT_CASE("rc06");
}

END_TEST()

FN_SETUP(cleanup)
{
	unmount_overlay_at(ROOT);
	CHECK(remove_tree(BASE_DIR));
}

END_SETUP()
