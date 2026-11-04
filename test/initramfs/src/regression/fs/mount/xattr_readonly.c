// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/xattr.h>
#include <unistd.h>

#include "../../common/test.h"

#define MOUNT_DIR "/tmp/xattr_readonly_mount"
#define TARGET MOUNT_DIR "/target"

static int target_fd;

FN_SETUP(prepare)
{
	CHECK(unshare(CLONE_NEWNS));
	CHECK(mount(NULL, "/", NULL, MS_PRIVATE | MS_REC, NULL));
	CHECK(mkdir(MOUNT_DIR, 0755));
	CHECK(mount("tmpfs", MOUNT_DIR, "tmpfs", 0, NULL));
	int fd = CHECK(open(TARGET, O_CREAT | O_EXCL | O_RDWR, 0644));
	CHECK(close(fd));
	CHECK(setxattr(TARGET, "user.set", "old", 3, 0));
	CHECK(setxattr(TARGET, "user.lset", "old", 3, 0));
	CHECK(setxattr(TARGET, "user.fset", "old", 3, 0));
	CHECK(setxattr(TARGET, "user.remove", "old", 3, 0));
	CHECK(setxattr(TARGET, "user.lremove", "old", 3, 0));
	CHECK(setxattr(TARGET, "user.fremove", "old", 3, 0));
	CHECK(mount(NULL, MOUNT_DIR, NULL, MS_REMOUNT | MS_RDONLY, NULL));
	target_fd = CHECK(open(TARGET, O_RDONLY));
}
END_SETUP()

FN_TEST(xattr_changes_on_readonly_mount)
{
	TEST_ERRNO(setxattr(TARGET, "user.set", "new", 3, 0), EROFS);
	TEST_ERRNO(lsetxattr(TARGET, "user.lset", "new", 3, 0), EROFS);
	TEST_ERRNO(fsetxattr(target_fd, "user.fset", "new", 3, 0), EROFS);
	TEST_ERRNO(removexattr(TARGET, "user.remove"), EROFS);
	TEST_ERRNO(lremovexattr(TARGET, "user.lremove"), EROFS);
	TEST_ERRNO(fremovexattr(target_fd, "user.fremove"), EROFS);

	const char *names[] = { "user.set",    "user.lset",    "user.fset",
				"user.remove", "user.lremove", "user.fremove" };
	for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); i++) {
		char value[3] = { 0 };
		ssize_t len = TEST_RES(getxattr(TARGET, names[i], value, 3),
				       _ret == 3);
		if (len == 3)
			TEST_RES(memcmp(value, "old", sizeof(value)),
				 _ret == 0);
	}
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(target_fd));
	CHECK(mount(NULL, MOUNT_DIR, NULL, MS_REMOUNT, NULL));
	CHECK(unlink(TARGET));
	CHECK(umount(MOUNT_DIR));
	CHECK(rmdir(MOUNT_DIR));
}
END_SETUP()
