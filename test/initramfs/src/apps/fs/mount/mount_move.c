// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <sched.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define MOVE_ROOT "/tmp/move_mount_root"
#define MOVE_CHILD "/tmp/move_mount_root/child"
#define MOVE_TARGET "/tmp/move_mount_root/child/target"

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

FN_TEST(move_mount_rejects_descendant_target)
{
	TEST_SUCC(unshare(CLONE_NEWNS));

	ensure_dir(MOVE_ROOT);
	TEST_SUCC(mount("tmpfs", MOVE_ROOT, "tmpfs", 0, NULL));

	ensure_dir(MOVE_CHILD);
	TEST_SUCC(mount("tmpfs", MOVE_CHILD, "tmpfs", 0, NULL));

	ensure_dir(MOVE_TARGET);

	TEST_ERRNO(mount(MOVE_ROOT, MOVE_ROOT, NULL, MS_MOVE, NULL), ELOOP);
	TEST_ERRNO(mount(MOVE_ROOT, MOVE_CHILD, NULL, MS_MOVE, NULL), ELOOP);
	TEST_ERRNO(mount(MOVE_ROOT, MOVE_TARGET, NULL, MS_MOVE, NULL), ELOOP);

	TEST_SUCC(umount(MOVE_CHILD));
	TEST_SUCC(umount(MOVE_ROOT));
}
END_TEST()
