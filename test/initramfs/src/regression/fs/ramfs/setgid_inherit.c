// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/tmp/ramfs_setgid_test"
#define FILE_PATH TEST_DIR "/newfile"
#define DIR_PATH TEST_DIR "/newdir"

#define DIR_GID 1000
#define CREATOR_GID 1001

/*
 * Files and directories created in a setgid directory inherit the
 * directory's gid. The subdirectory must also keep S_ISGID.
 *
 * The creator's egid differs from the parent directory's gid, so a
 * successful test proves inheritance rather than an accidental match.
 */
FN_TEST(setgid_directory_grants_gid_on_creation)
{
	TEST_SUCC(mkdir(TEST_DIR, 0755));
	TEST_SUCC(chown(TEST_DIR, 0, DIR_GID));
	TEST_SUCC(chmod(TEST_DIR, 02750));

	TEST_SUCC(setegid(CREATOR_GID));

	int fd = TEST_SUCC(creat(FILE_PATH, 0644));
	TEST_SUCC(close(fd));
	TEST_SUCC(mkdir(DIR_PATH, 0755));

	TEST_SUCC(setegid(0));

	struct stat file_st, dir_st;
	TEST_SUCC(stat(FILE_PATH, &file_st));
	TEST_SUCC(stat(DIR_PATH, &dir_st));
	TEST_RES(stat(FILE_PATH, &file_st), file_st.st_gid == DIR_GID);
	TEST_RES(stat(DIR_PATH, &dir_st),
		 dir_st.st_gid == DIR_GID && (dir_st.st_mode & S_ISGID) != 0);

	TEST_SUCC(unlink(FILE_PATH));
	TEST_SUCC(rmdir(DIR_PATH));
	TEST_SUCC(rmdir(TEST_DIR));
}
END_TEST()
