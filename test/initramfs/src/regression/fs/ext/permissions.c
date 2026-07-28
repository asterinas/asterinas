// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"
#include "fs_test.h"

#define BASE_DIR EXT_TEST_ROOT "/perm_test"
#define TEST_FILE BASE_DIR "/testfile"

FN_SETUP(create_base_dir)
{
	CHECK_WITH(mkdir(BASE_DIR, 0755), _ret == 0 || errno == EEXIST);
}
END_SETUP()

FN_TEST(chown_changes_owner)
{
	int fd = TEST_SUCC(creat(TEST_FILE, 0644));
	TEST_SUCC(close(fd));

	TEST_SUCC(chown(TEST_FILE, 1000, 1000));

	struct stat st;
	TEST_SUCC(stat(TEST_FILE, &st));
	TEST_RES(stat(TEST_FILE, &st), st.st_uid == 1000 && st.st_gid == 1000);

	// Restore ownership to root before cleanup
	TEST_SUCC(chown(TEST_FILE, 0, 0));
	TEST_SUCC(unlink(TEST_FILE));
}
END_TEST()

FN_TEST(chown_large_id)
{
	int fd = TEST_SUCC(creat(TEST_FILE, 0644));
	TEST_SUCC(close(fd));

	// A uid/gid beyond 16 bits exercises the inode's high-order owner
	// halves (the osd2 l_i_uid_high / l_i_gid_high fields), which even the
	// 128-byte inode layout carries. The value read back must stay intact.
	TEST_SUCC(chown(TEST_FILE, 100000, 200000));

	struct stat st;
	TEST_RES(stat(TEST_FILE, &st),
		 st.st_uid == 100000 && st.st_gid == 200000);

	// Restore ownership to root before cleanup
	TEST_SUCC(chown(TEST_FILE, 0, 0));
	TEST_SUCC(unlink(TEST_FILE));
}
END_TEST()

FN_TEST(chmod_changes_mode)
{
	int fd = TEST_SUCC(creat(TEST_FILE, 0644));
	TEST_SUCC(close(fd));

	TEST_SUCC(chmod(TEST_FILE, 0755));

	struct stat st;
	TEST_RES(stat(TEST_FILE, &st), (st.st_mode & 0777) == 0755);

	TEST_SUCC(unlink(TEST_FILE));
}
END_TEST()

FN_TEST(write_updates_mtime)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	struct stat st1;
	TEST_SUCC(stat(TEST_FILE, &st1));

	// Sleep to ensure the timestamp advances
	sleep(1);

	fd = TEST_SUCC(open(TEST_FILE, O_WRONLY));
	TEST_RES(write(fd, "hello", 5), _ret == 5);
	TEST_SUCC(close(fd));

	struct stat st2;
	TEST_RES(stat(TEST_FILE, &st2), st2.st_mtime > st1.st_mtime);

	TEST_SUCC(unlink(TEST_FILE));
}
END_TEST()
