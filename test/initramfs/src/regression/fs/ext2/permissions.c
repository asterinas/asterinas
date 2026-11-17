// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ext2/perm_test"
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

#define SUPP_GID 4242
#define FOREIGN_GID 4243
#define NON_OWNER_UID 1000

#define GROUP_RW_FILE BASE_DIR "/group_rw_other_none"
#define GROUP_R_FILE BASE_DIR "/group_r_other_w"
#define OTHER_W_FILE BASE_DIR "/foreign_group_other_w"

static void create_owned_file(const char *path, mode_t mode, gid_t gid)
{
	int fd = CHECK(creat(path, mode));
	CHECK(close(fd));
	CHECK(chown(path, 0, gid));
	/* creat() applies the umask, so set the final mode explicitly. */
	CHECK(chmod(path, mode));
}

static void open_should_succeed(const char *path, int flags)
{
	int fd = CHECK(open(path, flags));
	CHECK(close(fd));
}

static void open_should_be_denied(const char *path, int flags)
{
	CHECK_WITH(open(path, flags), _ret == -1 && errno == EACCES);
}

static void probe_group_class(void)
{
	/* The group class applies through a supplementary group: 0660 grants
	 * read and write, so both are allowed. */
	open_should_succeed(GROUP_RW_FILE, O_RDWR);

	/* 0642 withholds write from the group and grants it to others.
	 * Falling through to the other class would let a group member
	 * write. */
	open_should_be_denied(GROUP_R_FILE, O_WRONLY);

	/* A file in a group the caller does not belong to still uses the
	 * other class, so this control must keep succeeding. */
	open_should_succeed(OTHER_W_FILE, O_WRONLY);
}

static void run_in_child_with_supplementary_group(void)
{
	gid_t groups[] = { SUPP_GID };

	CHECK(syscall(SYS_setgroups, 1, groups));
	CHECK(setresgid(NON_OWNER_UID, NON_OWNER_UID, NON_OWNER_UID));
	CHECK(setresuid(NON_OWNER_UID, NON_OWNER_UID, NON_OWNER_UID));
	probe_group_class();
}

FN_TEST(supplementary_group_selects_group_class)
{
	create_owned_file(GROUP_RW_FILE, 0660, SUPP_GID);
	create_owned_file(GROUP_R_FILE, 0642, SUPP_GID);
	create_owned_file(OTHER_W_FILE, 0602, FOREIGN_GID);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		run_in_child_with_supplementary_group();
		_exit(0);
	}

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	TEST_SUCC(unlink(GROUP_RW_FILE));
	TEST_SUCC(unlink(GROUP_R_FILE));
	TEST_SUCC(unlink(OTHER_W_FILE));
}
END_TEST()
