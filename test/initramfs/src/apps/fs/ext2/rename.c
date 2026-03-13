// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ext2/rename_test"
#define DIR BASE_DIR "/dir"
#define DIR_RENAMED BASE_DIR "/dir_renamed"
#define DIR_CHILD DIR "/child"
#define DIR_GRANDCHILD DIR_CHILD "/grandchild"
#define DIR_TARGET DIR_GRANDCHILD "/moved"

#define DIR_RENAMED_CHILD DIR_RENAMED "/child"
#define DIR_RENAMED_GRANDCHILD DIR_RENAMED_CHILD "/grandchild"

#define CROSS_MOUNT_DIR BASE_DIR "/mnt"
#define CROSS_MOUNT_DIR_CHILD CROSS_MOUNT_DIR "/child"

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

static void remove_if_exists(const char *path)
{
	CHECK_WITH(rmdir(path), _ret == 0 || errno == ENOENT);
}

static void ensure_test_tree(void)
{
	ensure_dir(BASE_DIR);
	ensure_dir(DIR);
	ensure_dir(DIR_CHILD);
	ensure_dir(DIR_GRANDCHILD);
	ensure_dir(CROSS_MOUNT_DIR);
}

static void cleanup_test_tree(void)
{
	remove_if_exists(DIR_TARGET);
	remove_if_exists(DIR_RENAMED_GRANDCHILD);
	remove_if_exists(DIR_RENAMED_CHILD);
	remove_if_exists(DIR_GRANDCHILD);
	remove_if_exists(CROSS_MOUNT_DIR_CHILD);
	remove_if_exists(DIR_RENAMED);
	remove_if_exists(CROSS_MOUNT_DIR);
	remove_if_exists(DIR_CHILD);
	remove_if_exists(DIR);
	remove_if_exists(BASE_DIR);
}

FN_SETUP(cleanup_before_test)
{
	cleanup_test_tree();
}
END_SETUP()

FN_TEST(rename_into_descendant)
{
	ensure_test_tree();

	TEST_ERRNO(rename(DIR, DIR_TARGET), EINVAL);
	TEST_SUCC(access(DIR, F_OK));
	TEST_SUCC(access(DIR_GRANDCHILD, F_OK));

	cleanup_test_tree();
}
END_TEST()

FN_TEST(rename_to_self)
{
	ensure_test_tree();

	TEST_SUCC(rename(DIR, DIR));
	TEST_SUCC(access(DIR, F_OK));
	TEST_SUCC(access(DIR_GRANDCHILD, F_OK));

	cleanup_test_tree();
}
END_TEST()

FN_TEST(rename_to_new_name)
{
	ensure_test_tree();

	TEST_SUCC(rename(DIR, DIR_RENAMED));
	TEST_SUCC(access(DIR_RENAMED, F_OK));
	TEST_SUCC(access(DIR_RENAMED_CHILD, F_OK));
	TEST_ERRNO(access(DIR, F_OK), ENOENT);

	cleanup_test_tree();
}
END_TEST()

// On Linux, `renameat2` checks for mountpoint crossing before
// checking whether the new path is inside the old directory.
FN_TEST(rename_errno_order)
{
	ensure_test_tree();
	TEST_SUCC(mount("tmpfs", CROSS_MOUNT_DIR, "tmpfs", 0, NULL));
	ensure_dir(CROSS_MOUNT_DIR_CHILD);

	TEST_ERRNO(rename(DIR, CROSS_MOUNT_DIR_CHILD), EXDEV);
	TEST_SUCC(access(DIR, F_OK));
	TEST_SUCC(access(CROSS_MOUNT_DIR_CHILD, F_OK));

	TEST_SUCC(umount(CROSS_MOUNT_DIR));
	cleanup_test_tree();
}
END_TEST()
