// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#define BASE_DIR "/ovl_lchown_lower_symlink"

#include "ovl_common.h"

#define LINK_NAME "link"
#define LINK_TARGET "lower-symlink-target"
#define NEW_UID 1234
#define NEW_GID 1234

static void setup_overlay_tree(void)
{
	create_overlay_dirs();

	CHECK(symlink(LINK_TARGET, LOWER_DIR "/" LINK_NAME));
}

static void cleanup_overlay_tree(void)
{
	CHECK_WITH(unlink(UPPER_DIR "/" LINK_NAME),
		   _ret == 0 || errno == ENOENT);
	CHECK(unlink(LOWER_DIR "/" LINK_NAME));

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

FN_TEST(lchown_lower_symlink)
{
	char target[64] = { 0 };
	struct stat st;

	TEST_SUCC(lchown(MERGED_DIR "/" LINK_NAME, NEW_UID, NEW_GID));
	TEST_SUCC(lstat(MERGED_DIR "/" LINK_NAME, &st));
	TEST_RES(st.st_uid, _ret == NEW_UID);
	TEST_RES(st.st_gid, _ret == NEW_GID);
	TEST_RES(readlink(MERGED_DIR "/" LINK_NAME, target, sizeof(target)),
		 _ret == (ssize_t)strlen(LINK_TARGET));
	TEST_RES(strcmp(target, LINK_TARGET), _ret == 0);
}

END_TEST()

FN_SETUP(cleanup)
{
	CHECK(umount(MERGED_DIR));
	cleanup_overlay_tree();
}

END_SETUP()
