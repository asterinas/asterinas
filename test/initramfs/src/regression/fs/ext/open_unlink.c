// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/vfs.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"
#include "fs_test.h"

#define BASE_DIR EXT_TEST_ROOT "/open_unlink_test"
#define FILE_FORK BASE_DIR "/fork_file"
#define FILE_APPEND BASE_DIR "/append_file"

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

static void remove_file_if_exists(const char *path)
{
	CHECK_WITH(unlink(path), _ret == 0 || errno == ENOENT);
}

FN_SETUP(setup_base_dir)
{
	ensure_dir(BASE_DIR);
}
END_SETUP()

FN_TEST(fork_shares_fd_offset)
{
	remove_file_if_exists(FILE_FORK);

	int fd = TEST_SUCC(open(FILE_FORK, O_CREAT | O_RDWR, 0644));

	/* Parent writes "AAAA" at offset 0 */
	TEST_RES(write(fd, "AAAA", 4), _ret == 4);

	pid_t pid = TEST_RES(fork(), _ret >= 0);
	if (pid == 0) {
		/* Child: fd offset is 4, write "BB" */
		CHECK_WITH(write(fd, "BB", 2), _ret == 2);
		CHECK(close(fd));
		exit(0);
	}

	/* Parent: wait for child, then write "CC" at offset 6 */
	int status;
	TEST_SUCC(waitpid(pid, &status, 0));
	TEST_RES(write(fd, "CC", 2), _ret == 2);

	TEST_SUCC(close(fd));

	/* Reopen and verify contents */
	fd = TEST_SUCC(open(FILE_FORK, O_RDONLY));

	char buf[16] = { 0 };
	TEST_RES(read(fd, buf, sizeof(buf)), _ret == 8);
	TEST_RES(memcmp(buf, "AAAABBCC", 8), _ret == 0);

	TEST_SUCC(close(fd));
	remove_file_if_exists(FILE_FORK);
}
END_TEST()

FN_TEST(append_two_writers)
{
	remove_file_if_exists(FILE_APPEND);

	int fd = TEST_SUCC(
		open(FILE_APPEND, O_APPEND | O_CREAT | O_WRONLY, 0644));

	pid_t pid = TEST_RES(fork(), _ret >= 0);
	if (pid == 0) {
		for (int i = 0; i < 10; i++)
			CHECK_WITH(write(fd, "CHILD\n", 6), _ret == 6);
		CHECK(close(fd));
		exit(0);
	}

	for (int i = 0; i < 10; i++)
		TEST_RES(write(fd, "PARENT", 6), _ret == 6);

	int status;
	TEST_SUCC(waitpid(pid, &status, 0));
	TEST_SUCC(close(fd));

	/* Reopen and verify total length */
	fd = TEST_SUCC(open(FILE_APPEND, O_RDONLY));

	char buf[256] = { 0 };
	TEST_RES(read(fd, buf, sizeof(buf)), _ret == 120);

	TEST_SUCC(close(fd));
	remove_file_if_exists(FILE_APPEND);
}
END_TEST()

/* Closing the last unlinked handle must reclaim even unsynced allocations. */
FN_TEST(write_after_unlink_reclaims_unsynced_blocks)
{
	const char *path = BASE_DIR "/unsynced_file";
	remove_file_if_exists(path);
	struct statfs before, allocated, after;
	TEST_SUCC(statfs(BASE_DIR, &before));
	int fd = TEST_SUCC(open(path, O_CREAT | O_RDWR, 0644));
	TEST_SUCC(unlink(path));
	TEST_RES(write(fd, "alive", 5), _ret == 5);
	char buffer[5];
	TEST_RES(pread(fd, buffer, sizeof(buffer), 0), _ret == sizeof(buffer));
	TEST_RES(memcmp(buffer, "alive", sizeof(buffer)), _ret == 0);
	TEST_SUCC(statfs(BASE_DIR, &allocated));
	TEST_RES(allocated.f_bfree, _ret < before.f_bfree);
	TEST_SUCC(close(fd));
	TEST_SUCC(statfs(BASE_DIR, &after));
	TEST_RES(after.f_bfree, _ret == before.f_bfree);
	TEST_RES(after.f_ffree, _ret == before.f_ffree);
}
END_TEST()
