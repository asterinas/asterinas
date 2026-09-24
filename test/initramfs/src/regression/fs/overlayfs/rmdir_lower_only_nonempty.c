// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_rmdir_lower_only"

#include "ovl_common.h"

static void setup_overlay_tree(void)
{
	create_overlay_dirs();

	create_dir(LOWER_DIR "/target");
	write_file(LOWER_DIR "/target/.wh.foo", "data");
}

static void cleanup_overlay_tree(void)
{
	CHECK(unlink(LOWER_DIR "/target/.wh.foo"));
	CHECK(rmdir(LOWER_DIR "/target"));

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
