// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/vfs.h>
#include <sys/xattr.h>
#include <unistd.h>

#include "../../common/test.h"
#include "fs_test.h"

#define BASE_DIR EXT_TEST_ROOT "/xattr_test"

static void ensure_base_dir(void)
{
	CHECK_WITH(mkdir(BASE_DIR, 0755), _ret >= 0 || errno == EEXIST);
}

FN_SETUP(prepare_base_dir)
{
	ensure_base_dir();
}
END_SETUP()

FN_TEST(xattr_set_get_roundtrip)
{
	const char *path = BASE_DIR "/roundtrip";
	int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	TEST_SUCC(setxattr(path, "user.test", "value123", 8, 0));

	char buf[64] = { 0 };
	TEST_RES(getxattr(path, "user.test", buf, sizeof(buf)), _ret == 8);
	TEST_RES(strcmp(buf, "value123"), _ret == 0);

	TEST_SUCC(unlink(path));
}
END_TEST()

// Currently, the `listxattr()` syscall in Asterinas only lists xattrs within
// one namespace ("Trusted" when running as root), so `user.*` xattrs are
// invisible. This is a VFS-level bug, not ext2-specific.
//
// TODO: Add some tests for xattr listing after this bug has been fixed.

FN_TEST(xattr_remove_enodata)
{
	const char *path = BASE_DIR "/removefile";
	int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	TEST_SUCC(setxattr(path, "user.gone", "tmp", 3, 0));
	TEST_SUCC(removexattr(path, "user.gone"));

	char buf[64] = { 0 };
	TEST_ERRNO(getxattr(path, "user.gone", buf, sizeof(buf)), ENODATA);

	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(xattr_ea_block_alloc_and_release)
{
	const char *path = BASE_DIR "/eablock";
	int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	// Extended attributes always live in a separately allocated EA block
	// (there is no in-inode xattr storage), so the first attribute consumes
	// one filesystem block and removing the last one frees it. The EA block
	// is allocated outside the inode's own block accounting, so it is
	// observed through the volume's free-block count -- a relative delta
	// that holds on every image shape, including the 128-byte-inode and
	// ext2 cells -- rather than the file's st_blocks.
	struct statfs before;
	TEST_SUCC(statfs(EXT_TEST_ROOT, &before));

	TEST_SUCC(setxattr(path, "user.eablock", "v", 1, 0));

	struct statfs after_set;
	TEST_RES(statfs(EXT_TEST_ROOT, &after_set),
		 after_set.f_bfree == before.f_bfree - 1);

	TEST_SUCC(removexattr(path, "user.eablock"));

	struct statfs after_remove;
	TEST_RES(statfs(EXT_TEST_ROOT, &after_remove),
		 after_remove.f_bfree == before.f_bfree);

	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(xattr_nonexistent_enodata)
{
	const char *path = BASE_DIR "/noattr";
	int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	char buf[64] = { 0 };
	TEST_ERRNO(getxattr(path, "user.nonexistent", buf, sizeof(buf)),
		   ENODATA);

	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(xattr_with_o_path_fd)
{
	const char *path = BASE_DIR "/opath";
	const char name[] = "user.test";
	int val = 1234;
	char buf[sizeof(val)];
	char list[sizeof(name)];

	int fd = TEST_SUCC(open(path, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	int opath_fd = TEST_SUCC(open(path, O_PATH));
	TEST_ERRNO(syscall(SYS_fsetxattr, opath_fd, name, &val, sizeof(val), 0),
		   EBADF);
	TEST_ERRNO(syscall(SYS_fgetxattr, opath_fd, name, buf, sizeof(buf)),
		   EBADF);
	TEST_ERRNO(syscall(SYS_flistxattr, opath_fd, list, sizeof(list)),
		   EBADF);
	TEST_ERRNO(syscall(SYS_fremovexattr, opath_fd, name), EBADF);
	TEST_SUCC(close(opath_fd));

	TEST_SUCC(unlink(path));
}
END_TEST()
