// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"
#include "fs_test.h"

#define BASE_DIR EXT_TEST_ROOT "/rename_test"
#define DIR BASE_DIR "/dir"
#define DIR_RENAMED BASE_DIR "/dir_renamed"
#define DIR_CHILD DIR "/child"
#define DIR_GRANDCHILD DIR_CHILD "/grandchild"
#define DIR_TARGET DIR_GRANDCHILD "/moved"

#define DIR_RENAMED_CHILD DIR_RENAMED "/child"
#define DIR_RENAMED_GRANDCHILD DIR_RENAMED_CHILD "/grandchild"
#define FILE_A BASE_DIR "/A"
#define FILE_B BASE_DIR "/B"

#define CROSS_MOUNT_DIR BASE_DIR "/mnt"
#define CROSS_MOUNT_DIR_CHILD CROSS_MOUNT_DIR "/child"

FN_SETUP(prepare_base_dir)
{
	CHECK(mkdir(BASE_DIR, 0755));
}
END_SETUP()

#define PREPARE_TREE()                                  \
	do {                                            \
		TEST_SUCC(mkdir(DIR, 0755));            \
		TEST_SUCC(mkdir(DIR_CHILD, 0755));      \
		TEST_SUCC(mkdir(DIR_GRANDCHILD, 0755)); \
	} while (0)

#define REMOVE_TREE(root, child, grandchild)  \
	do {                                  \
		TEST_SUCC(rmdir(grandchild)); \
		TEST_SUCC(rmdir(child));      \
		TEST_SUCC(rmdir(root));       \
	} while (0)

FN_TEST(rename_into_descendant)
{
	PREPARE_TREE();

	TEST_ERRNO(rename(DIR, DIR_TARGET), EINVAL);
	TEST_SUCC(access(DIR, F_OK));
	TEST_SUCC(access(DIR_GRANDCHILD, F_OK));

	REMOVE_TREE(DIR, DIR_CHILD, DIR_GRANDCHILD);
}
END_TEST()

FN_TEST(rename_to_self)
{
	PREPARE_TREE();

	TEST_SUCC(rename(DIR, DIR));
	TEST_SUCC(access(DIR, F_OK));
	TEST_SUCC(access(DIR_GRANDCHILD, F_OK));

	REMOVE_TREE(DIR, DIR_CHILD, DIR_GRANDCHILD);
}
END_TEST()

FN_TEST(rename_to_new_name)
{
	PREPARE_TREE();

	TEST_SUCC(rename(DIR, DIR_RENAMED));
	TEST_SUCC(access(DIR_RENAMED, F_OK));
	TEST_SUCC(access(DIR_RENAMED_CHILD, F_OK));
	TEST_ERRNO(access(DIR, F_OK), ENOENT);

	REMOVE_TREE(DIR_RENAMED, DIR_RENAMED_CHILD, DIR_RENAMED_GRANDCHILD);
}
END_TEST()

FN_TEST(rename_overwrites_negative_cache_on_ext2)
{
	int fd_a = TEST_SUCC(open(FILE_A, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd_a));
	int fd_b = TEST_SUCC(open(FILE_B, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd_b));

	TEST_SUCC(unlink(FILE_B));
	TEST_ERRNO(access(FILE_B, F_OK), ENOENT);

	TEST_SUCC(rename(FILE_A, FILE_B));
	TEST_SUCC(access(FILE_B, F_OK));
	TEST_SUCC(access(FILE_B, F_OK));

	TEST_SUCC(unlink(FILE_B));
}
END_TEST()

// On Linux, `renameat2` checks for mountpoint crossing before
// checking whether the new path is inside the old directory.
FN_TEST(rename_errno_order)
{
	PREPARE_TREE();
	TEST_SUCC(mkdir(CROSS_MOUNT_DIR, 0755));
	TEST_SUCC(mount("tmpfs", CROSS_MOUNT_DIR, "tmpfs", 0, NULL));
	TEST_SUCC(mkdir(CROSS_MOUNT_DIR_CHILD, 0755));

	TEST_ERRNO(rename(DIR, CROSS_MOUNT_DIR_CHILD), EXDEV);
	TEST_SUCC(access(DIR, F_OK));
	TEST_SUCC(access(CROSS_MOUNT_DIR_CHILD, F_OK));

	TEST_SUCC(rmdir(CROSS_MOUNT_DIR_CHILD));
	TEST_SUCC(umount(CROSS_MOUNT_DIR));
	TEST_SUCC(rmdir(CROSS_MOUNT_DIR));
	REMOVE_TREE(DIR, DIR_CHILD, DIR_GRANDCHILD);
}
END_TEST()

FN_TEST(rename_dir_replace_nlink)
{
	const char *src_parent = BASE_DIR "/src_parent";
	const char *src_child = BASE_DIR "/src_parent/child";
	const char *dst_parent = BASE_DIR "/dst_parent";
	const char *dst_child = BASE_DIR "/dst_parent/child";

	TEST_SUCC(mkdir(src_parent, 0755));
	TEST_SUCC(mkdir(src_child, 0755));
	TEST_SUCC(mkdir(dst_parent, 0755));
	TEST_SUCC(mkdir(dst_child, 0755));

	struct stat st_before;
	TEST_SUCC(stat(dst_parent, &st_before));

	TEST_SUCC(rename(src_child, dst_child));

	struct stat st_after;
	TEST_RES(stat(dst_parent, &st_after),
		 st_after.st_nlink == st_before.st_nlink);

	TEST_SUCC(rmdir(dst_child));
	TEST_SUCC(rmdir(dst_parent));
	TEST_SUCC(rmdir(src_parent));
}
END_TEST()

FN_SETUP(cleanup_base_dir)
{
	CHECK(rmdir(BASE_DIR));
}
END_SETUP()
