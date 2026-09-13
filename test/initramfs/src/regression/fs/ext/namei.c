// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"
#include "fs_test.h"

#define BASE_DIR EXT_TEST_ROOT "/namei_test"

#define DIR_A BASE_DIR "/a"
#define DIR_B BASE_DIR "/b"

FN_SETUP(prepare_base_dir)
{
	CHECK(mkdir(BASE_DIR, 0755));
}
END_SETUP()

/* creat(".") should fail with EISDIR */
FN_TEST(create_dot_eisdir)
{
	TEST_ERRNO(creat(BASE_DIR "/.", 0644), EISDIR);
}
END_TEST()

/* rename a file across directories */
FN_TEST(rename_cross_dir)
{
	const char *old_path = DIR_A "/move_me";
	const char *new_path = DIR_B "/move_me";

	TEST_SUCC(mkdir(DIR_A, 0755));
	TEST_SUCC(mkdir(DIR_B, 0755));

	int fd = TEST_SUCC(creat(old_path, 0644));
	TEST_SUCC(close(fd));

	TEST_SUCC(rename(old_path, new_path));

	TEST_ERRNO(access(old_path, F_OK), ENOENT);
	TEST_SUCC(access(new_path, F_OK));

	TEST_SUCC(unlink(new_path));
	TEST_SUCC(rmdir(DIR_B));
	TEST_SUCC(rmdir(DIR_A));
}
END_TEST()

/* rename replaces an existing file in another directory */
FN_TEST(rename_cross_dir_replace)
{
	const char *src = DIR_A "/replace_file";
	const char *dst = DIR_B "/replace_file";

	TEST_SUCC(mkdir(DIR_A, 0755));
	TEST_SUCC(mkdir(DIR_B, 0755));

	const char src_data[] = "source_data";
	int fd = TEST_SUCC(open(src, O_CREAT | O_WRONLY, 0644));
	TEST_RES(write(fd, src_data, sizeof(src_data)),
		 _ret == (ssize_t)sizeof(src_data));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(creat(dst, 0644));
	TEST_SUCC(close(fd));

	TEST_SUCC(rename(src, dst));

	TEST_ERRNO(access(src, F_OK), ENOENT);

	char buf[64] = { 0 };
	fd = TEST_SUCC(open(dst, O_RDONLY));
	TEST_RES(read(fd, buf, sizeof(buf)),
		 _ret == sizeof(src_data) &&
			 memcmp(buf, src_data, sizeof(src_data)) == 0);
	TEST_SUCC(close(fd));

	TEST_SUCC(unlink(dst));
	TEST_SUCC(rmdir(DIR_B));
	TEST_SUCC(rmdir(DIR_A));
}
END_TEST()

/* rename a directory updates ".." to point to the new parent */
FN_TEST(rename_dir_dotdot)
{
	const char *parent = BASE_DIR "/old_parent";
	const char *child = BASE_DIR "/old_parent/child";
	const char *new_parent = BASE_DIR "/new_parent";
	const char *new_child = BASE_DIR "/new_parent/child";

	TEST_SUCC(mkdir(parent, 0755));
	TEST_SUCC(mkdir(child, 0755));
	TEST_SUCC(mkdir(new_parent, 0755));

	struct stat st_new_parent;
	TEST_SUCC(stat(new_parent, &st_new_parent));

	TEST_SUCC(rename(child, new_child));

	struct stat st_dotdot;
	const char *dotdot_path = BASE_DIR "/new_parent/child/..";
	TEST_RES(stat(dotdot_path, &st_dotdot),
		 st_dotdot.st_ino == st_new_parent.st_ino);

	TEST_SUCC(rmdir(new_child));
	TEST_SUCC(rmdir(new_parent));
	TEST_SUCC(rmdir(parent));
}
END_TEST()

/* open + O_CREAT + O_DIRECTORY on existing directory edge cases */

#define OPEN_DIR_TEST_DIR BASE_DIR "/open_dir_test"
#define OPEN_DIR_TARGET OPEN_DIR_TEST_DIR "/objd"

FN_SETUP(prepare_open_dir)
{
	CHECK(mkdir(OPEN_DIR_TEST_DIR, 0755));
	CHECK(mkdir(OPEN_DIR_TARGET, 0755));
}
END_SETUP()

FN_TEST(open_creat_dir_einval)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET, O_CREAT | O_DIRECTORY | O_RDONLY,
			0644),
		   EINVAL);
}
END_TEST()

FN_TEST(open_creat_excl_dir_einval)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET,
			O_CREAT | O_EXCL | O_DIRECTORY | O_WRONLY, 0644),
		   EINVAL);
}
END_TEST()

FN_TEST(open_creat_excl_dir_rdwr_einval)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET,
			O_CREAT | O_EXCL | O_DIRECTORY | O_RDWR, 0644),
		   EINVAL);
}
END_TEST()

FN_TEST(open_creat_rdonly_dir_eisdir)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET, O_CREAT | O_RDONLY, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_creat_wr_dir_eisdir)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET, O_CREAT | O_WRONLY, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_creat_rdwr_dir_eisdir)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET, O_CREAT | O_RDWR, 0644), EISDIR);
}
END_TEST()

FN_TEST(open_creat_excl_rdonly_eexist)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET, O_CREAT | O_EXCL | O_RDONLY, 0644),
		   EEXIST);
}
END_TEST()

FN_TEST(open_creat_excl_wr_eexist)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET, O_CREAT | O_EXCL | O_WRONLY, 0644),
		   EEXIST);
}
END_TEST()

FN_TEST(open_creat_excl_rdwr_eexist)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET, O_CREAT | O_EXCL | O_RDWR, 0644),
		   EEXIST);
}
END_TEST()

FN_TEST(open_wr_dir_eisdir)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET, O_WRONLY), EISDIR);
}
END_TEST()

FN_TEST(open_rdwr_dir_eisdir)
{
	TEST_ERRNO(open(OPEN_DIR_TARGET, O_RDWR), EISDIR);
}
END_TEST()

FN_SETUP(cleanup_open_dir)
{
	CHECK(rmdir(OPEN_DIR_TARGET));
	CHECK(rmdir(OPEN_DIR_TEST_DIR));
}
END_SETUP()

FN_SETUP(cleanup_base_dir)
{
	CHECK(rmdir(BASE_DIR));
}
END_SETUP()
