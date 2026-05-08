// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define FIFO_PATH "/tmp/proc-fd-open-fifo-after-setid"

FN_TEST(open_fifo_via_proc_fd_after_setid)
{
	char proc_fd_path[64];
	int fifo_fd;
	pid_t child;
	int writer_fd;
	int status;
	char buf = 0;

	TEST_SUCC(mkfifo(FIFO_PATH, 0644));

	fifo_fd = TEST_SUCC(open(FIFO_PATH, O_PATH));
	snprintf(proc_fd_path, sizeof(proc_fd_path), "/proc/self/fd/%d",
		 fifo_fd);

	child = TEST_SUCC(fork());
	if (child == 0) {
		int proc_fd;
		int reopened_fd;
		struct stat proc_dir_stat;
		struct stat proc_stat;

		CHECK(setresgid(-1, 65535, -1));
		CHECK(setresuid(-1, 65535, -1));

		/*
		 * FIXME: Asterinas does not support the "dumpable" attribute
		 * yet. So procfs files are always owned by the user
		 * corresponding to the thread's effective UID.
		 *
		 * When resolving this FIXME, keep in mind that if the
		 * "dumpable" attribute is not set, the procfs fd entry below
		 * will be owned by the root user. However, the re-open should
		 * still succeed because Linux uses special logic to bypass
		 * the permission check.
		 */
#ifdef __asterinas__
		CHECK(prctl(PR_SET_DUMPABLE, 1));
		CHECK_WITH(prctl(PR_GET_DUMPABLE), _ret != 1);
#else
		CHECK_WITH(prctl(PR_GET_DUMPABLE), _ret != 1);
		CHECK_WITH(stat("/proc/self/fd", &proc_dir_stat),
			   proc_dir_stat.st_uid == 0 &&
				   proc_dir_stat.st_gid == 0);
		CHECK(prctl(PR_SET_DUMPABLE, 1));
		CHECK_WITH(prctl(PR_GET_DUMPABLE), _ret == 1);
#endif

		CHECK_WITH(stat("/proc/self/fd", &proc_dir_stat),
			   proc_dir_stat.st_uid == 65535 &&
				   proc_dir_stat.st_gid == 65535);

		// Open the procfs fd entry itself. With `O_PATH | O_NOFOLLOW`, `fstat`
		// reports the owner of `/proc/self/fd/<n>` instead of the FIFO target.
		proc_fd = CHECK(open(proc_fd_path, O_PATH | O_NOFOLLOW));
		CHECK_WITH(fstat(proc_fd, &proc_stat),
			   proc_stat.st_uid == 65535 &&
				   proc_stat.st_gid == 65535);
		CHECK(close(proc_fd));

		// The FIFO target is owned by the root user, yet it is world-readable.
		// So it can be re-opened.
		reopened_fd = CHECK(open(proc_fd_path, O_RDONLY));
		CHECK_WITH(fstat(reopened_fd, &proc_stat),
			   proc_stat.st_uid == 0 && proc_stat.st_gid == 0);

		CHECK_WITH(read(reopened_fd, &buf, 1), _ret == 1 && buf == 'X');
		CHECK(close(reopened_fd));

		exit(EXIT_SUCCESS);
	}

	writer_fd = TEST_SUCC(open(FIFO_PATH, O_WRONLY));
	TEST_RES(write(writer_fd, "X", 1), _ret == 1);
	TEST_SUCC(close(writer_fd));

	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);

	TEST_SUCC(close(fifo_fd));
	TEST_SUCC(unlink(FIFO_PATH));
}
END_TEST()
