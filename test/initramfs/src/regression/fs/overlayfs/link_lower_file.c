// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_link_lower_file"

#include "ovl_common.h"

#define SOURCE_NAME "source"
#define TARGET_DIR_NAME "target"
#define LINK_NAME "link"

static void setup_overlay_tree(void)
{
	create_overlay_dirs();

	write_file(LOWER_DIR "/" SOURCE_NAME, "link-source-data");
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

	remove_overlay_dirs();
	CHECK(rmdir(LOWER_DIR));
	CHECK(rmdir(BASE_DIR));
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
